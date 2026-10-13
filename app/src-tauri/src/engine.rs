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

/// The desktop exposes extension operations only; account operations are retired.
const READ_WRITE_DOMAINS: &[&str] = &["mcp", "skills", "rules"];

pub fn validate_command(command: &str) -> Result<(), EngineError> {
    let Some((domain, verb)) = command.split_once(':') else {
        return Err(EngineError::new(
            "unknown_command",
            "命令必须使用 domain:verb 形式",
        ));
    };
    let allowed = match (domain, verb) {
        (
            "launch",
            "build" | "status" | "preview" | "apply" | "catalog" | "run" | "list" | "save"
            | "delete",
        ) => true,
        ("resources", "targets") => true,
        ("system", "audit" | "discover" | "refresh-upstream") => true,
        ("secrets", "status" | "set" | "source") => true,
        ("runs", "list" | "get" | "clear" | "progress") => true,
        ("policy", "get" | "set" | "delete") => true,
        ("db", "status" | "migrate") => true,
        ("config", "export" | "import") => true,
        ("rules", "save-template" | "delete-template") => true,
        ("mcp", "preview" | "check") => true,
        ("skills", "fork" | "versions" | "preview" | "adopt-preview" | "adopt") => true,
        ("programs", "status" | "preview" | "apply" | "declare") => true,
        (d, "status" | "apply") => READ_WRITE_DOMAINS.contains(&d),
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(EngineError::new(
            "unknown_command",
            "本工具仅允许 MCP、规则、技能及其扩展依赖操作",
        ))
    }
}

/// 参数取值类型（按类型校验；不经 shell，argv 直传引擎）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArgValueKind {
    LaunchShell,
    LaunchName,
    /// 纯开关（如 --allow-uninstall），不带值。
    Flag,
    /// 元素 id，如 mcp:example-search / skill:agent-skills/find-skills。
    ElementId,
    /// 运行 id（runs:get 的 --run），字母数字与 _/-。
    RunId,
    /// policy:set 落点层。
    PolicyLayer,
    /// policy:set 点分键路径（对齐引擎 key 段规则：字母/数字/_/-）。
    DotKey,
    /// policy:set 的 JSON 值——合法性即 JSON 可解析。
    JsonValue,
    SkillAction,
    McpAction,
    /// secrets:set 的变量名：与 ${VAR} 占位符同形。
    VarName,
    /// secrets:set 的取值：不校验内容（任何密钥形态都可能），只挡控制字符；
    /// 取值绝不进日志与诊断。
    SecretValue,
    /// 用户选择的外部密钥文件路径。允许空值表示停止使用，但拒绝控制字符与超长输入。
    FilePath,
    /// 写入基准：status 给出的 64 位小写十六进制摘要（agents:configure 的 --base；
    /// mcp:apply 的 --base / --to-base）。
    Digest,
    /// 规则正文（整份文件内容）：任意文本，换行等照常；NUL 等内容规则由引擎判定。
    RuleText,
    /// 规则、Hooks 与终端快捷命令写入的 CAS 基准：64 位小写 sha256，或没有内容时的 missing。
    RuleBase,
    /// Hooks / 终端快捷命令的建议 id：cleanup-/legacy- 前缀加 16 位小写十六进制
    /// （引擎再校验前缀与来源）。
    SuggestionId,
    /// 规则模板 id：小写字母、数字与连字符。
    TemplateId,
    /// 来源规则 id：`<集合>/<名字>`，两段只含字母、数字与 `-_.`，且不是 `.`/`..`。
    SourceRuleId,
    /// 规则模板名称：不含控制字符（换行也不行），长度有上限。
    TemplateName,
    /// 规则模板 revision：64 位小写 sha256。
    Revision,
    /// profiles:seed 要带的项：逗号分隔，只能是引擎认得的几项（引擎再校验一次）。
    SeedItems,
    /// 技能来源要切换到的版本：标签、分支或提交（字母数字与 `._/+-`，不以 `-` 开头）。
    GitRef,
    /// 逗号分隔的账号 id：每个只含字母数字与 `-_.`，至少一个。
    AgentList,
    /// 程序动作：install / upgrade / switch / restore / remove / adopt / unadopt / link / unlink。
    ProgramAction,
    /// true / false
    BoolText,
    AccountOperation,
    AccountShell,
}

/// 每个命令允许的 flag 集合（命令感知白名单）。
fn allowed_arg_specs(command: &str) -> Vec<(&'static str, ArgValueKind)> {
    let (domain, verb) = command.split_once(':').unwrap_or(("", ""));
    let mut specs: Vec<(&'static str, ArgValueKind)> = Vec::new();
    match (domain, verb) {
        ("launch", "build") => specs.push(("--spec", ArgValueKind::JsonValue)),
        ("launch", "save" | "delete") => {
            if verb == "save" {
                specs.push(("--spec", ArgValueKind::JsonValue));
            }
            specs.push(("--id", ArgValueKind::TemplateId));
            specs.push(("--base", ArgValueKind::Digest));
        }
        ("launch", "run") => {
            specs.push(("--spec", ArgValueKind::JsonValue));
            specs.push(("--name", ArgValueKind::LaunchName));
            specs.push(("--confirm", ArgValueKind::Flag));
        }
        ("launch", "status") => specs.push(("--shell", ArgValueKind::LaunchShell)),
        ("launch", "preview" | "apply") => {
            specs.push(("--spec", ArgValueKind::JsonValue));
            specs.push(("--shell", ArgValueKind::LaunchShell));
            specs.push(("--base", ArgValueKind::RuleBase));
            specs.push(("--remove", ArgValueKind::LaunchName));
            if verb == "apply" {
                specs.push(("--confirm", ArgValueKind::Flag));
            }
        }
        ("accounts", "command") => {
            specs.push(("--only", ArgValueKind::ElementId));
            specs.push(("--operation", ArgValueKind::AccountOperation));
            specs.push(("--definition", ArgValueKind::JsonValue));
            specs.push(("--shell", ArgValueKind::AccountShell));
        }
        ("profiles", "create") => {
            specs.push(("--only", ArgValueKind::ElementId));
            specs.push(("--definition", ArgValueKind::JsonValue));
            specs.push(("--base", ArgValueKind::RuleBase));
            specs.push(("--preview-file-base", ArgValueKind::RuleBase));
            specs.push(("--preview-content-base", ArgValueKind::RuleBase));
            specs.push(("--secret-var", ArgValueKind::VarName));
            specs.push(("--secret-value", ArgValueKind::SecretValue));
        }
        ("secrets", "set") => {
            specs.push(("--var", ArgValueKind::VarName));
            specs.push(("--value", ArgValueKind::SecretValue));
        }
        // 选用外部密钥文件；--create 在文件不存在时以 0600 新建一个空的（引擎再校验路径）
        ("secrets", "source") => {
            specs.push(("--path", ArgValueKind::FilePath));
            specs.push(("--create", ArgValueKind::Flag));
        }
        ("system", "refresh-upstream") => specs.push(("--only", ArgValueKind::ElementId)),
        ("programs", "status") => specs.push(("--rescan", ArgValueKind::Flag)),
        ("agents", "status") => specs.push(("--metadata-only", ArgValueKind::Flag)),
        ("skills", "status") => specs.push(("--collections-only", ArgValueKind::Flag)),
        ("programs", "preview" | "apply") => {
            specs.push(("--program", ArgValueKind::ElementId));
            specs.push(("--action", ArgValueKind::ProgramAction));
            specs.push(("--version", ArgValueKind::GitRef));
            specs.push(("--backup", ArgValueKind::GitRef));
            specs.push(("--entry-dir", ArgValueKind::FilePath));
            specs.push(("--allow-scripts", ArgValueKind::BoolText));
        }
        ("programs", "declare") => {
            specs.push(("--skill", ArgValueKind::GitRef));
            specs.push(("--bins", ArgValueKind::JsonValue));
            specs.push(("--bin", ArgValueKind::GitRef));
            specs.push(("--program", ArgValueKind::ElementId));
            specs.push(("--allow-scripts", ArgValueKind::BoolText));
            specs.push(("--server", ArgValueKind::GitRef));
            specs.push(("--follow-latest", ArgValueKind::BoolText));
        }
        // 规则以实际文件为准：写一个目标，base 做 CAS；新内容四选一（引擎再强制恰好一个）
        ("rules", "apply") => {
            specs.push(("--only", ArgValueKind::ElementId));
            specs.push(("--base", ArgValueKind::RuleBase));
            specs.push(("--value", ArgValueKind::RuleText));
            specs.push(("--template", ArgValueKind::TemplateId));
            specs.push(("--source-rule", ArgValueKind::SourceRuleId));
            specs.push(("--from", ArgValueKind::ElementId));
        }
        ("rules", "save-template") => {
            specs.push(("--name", ArgValueKind::TemplateName));
            specs.push(("--value", ArgValueKind::RuleText));
            specs.push(("--template", ArgValueKind::TemplateId));
            specs.push(("--revision", ArgValueKind::Revision));
        }
        ("rules", "delete-template") => specs.push(("--template", ArgValueKind::TemplateId)),
        // 终端快捷命令以 rc 文件受管区块为准：按名编辑一条，要写的文件各带自己的 base 做 CAS；
        // 操作四选一（引擎再强制恰好一个）。名称与元素 id 同一把尺子，取值由引擎校验。
        ("aliases", "apply" | "preview") => {
            specs.push(("--zshrc-base", ArgValueKind::RuleBase));
            specs.push(("--zprofile-base", ArgValueKind::RuleBase));
            // bash 用户（多数 Linux）的 ~/.bashrc
            specs.push(("--bashrc-base", ArgValueKind::RuleBase));
            // Windows：命令脚本目录的写入基准
            specs.push(("--cmd-base", ArgValueKind::RuleBase));
            specs.push(("--set", ArgValueKind::ElementId));
            specs.push(("--value", ArgValueKind::RuleText));
            specs.push(("--rename-from", ArgValueKind::ElementId));
            specs.push(("--remove", ArgValueKind::ElementId));
            specs.push(("--recommended", ArgValueKind::ElementId));
            specs.push(("--legacy", ArgValueKind::SuggestionId));
        }
        ("aliases", "discard-legacy") => specs.push(("--legacy", ArgValueKind::SuggestionId)),
        // 以实际配置更新实例定义：同样带基准，确认采纳的就是用户看到的那份实际内容
        ("profiles", "adopt") => {
            specs.push(("--only", ArgValueKind::ElementId));
            specs.push(("--base", ArgValueKind::RuleBase));
        }
        ("profiles", "apply-definition") => {
            specs.push(("--only", ArgValueKind::ElementId));
            specs.push(("--base", ArgValueKind::RuleBase));
            specs.push(("--definition-base", ArgValueKind::Digest));
            specs.push(("--candidate", ArgValueKind::JsonValue));
            specs.push(("--preview-file-base", ArgValueKind::RuleBase));
            specs.push(("--preview-content-base", ArgValueKind::Digest));
            specs.push(("--secret-var", ArgValueKind::VarName));
            specs.push(("--secret-value", ArgValueKind::SecretValue));
        }
        // 只读发现：--only 取域名（agents/skills/mcp），引擎侧再做词汇表校验
        ("profiles", "seed") => {
            specs.push(("--only", ArgValueKind::ElementId));
            specs.push(("--items", ArgValueKind::SeedItems));
        }
        ("profiles", "shared-status") => specs.push(("--only", ArgValueKind::ElementId)),
        ("profiles", "preview") => {
            specs.push(("--only", ArgValueKind::ElementId));
            specs.push(("--definition", ArgValueKind::JsonValue));
            specs.push(("--on-save", ArgValueKind::Flag));
            specs.push(("--items", ArgValueKind::SeedItems));
            specs.push(("--base", ArgValueKind::RuleBase));
            specs.push(("--definition-base", ArgValueKind::Digest));
            specs.push(("--candidate", ArgValueKind::JsonValue));
            specs.push(("--exact", ArgValueKind::Flag));
            specs.push(("--preview-secret-var", ArgValueKind::VarName));
            specs.push(("--preview-secret-value", ArgValueKind::SecretValue));
        }
        ("profiles", "shared-apply") => {
            specs.push(("--only", ArgValueKind::ElementId));
            specs.push(("--item", ArgValueKind::ElementId));
            specs.push(("--source-base", ArgValueKind::RuleBase));
            specs.push(("--target-base", ArgValueKind::RuleBase));
        }
        ("system", "discover") => specs.push(("--only", ArgValueKind::ElementId)),
        ("runs", "get" | "progress") => {
            specs.push(("--run", ArgValueKind::RunId));
        }
        ("policy", "set") => {
            specs.push(("--layer", ArgValueKind::PolicyLayer));
            specs.push(("--key", ArgValueKind::DotKey));
            specs.push(("--value", ArgValueKind::JsonValue));
            specs.push(("--expect-definition", ArgValueKind::JsonValue));
        }
        ("policy", "delete") => {
            specs.push(("--layer", ArgValueKind::PolicyLayer));
            specs.push(("--key", ArgValueKind::DotKey));
        }
        // 配置备份：导出不带参数，导入只接整份备份正文
        ("config", "import") => {
            specs.push(("--value", ArgValueKind::JsonValue));
            specs.push(("--modules", ArgValueKind::AgentList));
            specs.push(("--base", ArgValueKind::Digest));
        }
        ("skills", "fork") => {
            specs.push(("--only", ArgValueKind::ElementId));
            specs.push(("--dest", ArgValueKind::FilePath));
            specs.push(("--collection", ArgValueKind::ElementId));
        }
        ("mcp", "check") => {
            specs.push(("--only", ArgValueKind::ElementId));
            specs.push(("--agent", ArgValueKind::ElementId));
            specs.push(("--base", ArgValueKind::Digest));
            specs.push(("--cwd", ArgValueKind::FilePath));
        }
        ("agents", "auth-status") => {
            specs.push(("--only", ArgValueKind::ElementId));
            // 用户点「确认」时才带：用官方命令查
            specs.push(("--probe", ArgValueKind::Flag));
        }
        // 运行参数写助手配置文件：每次写入都带 status 给出的基准，外部改过就被拒；
        // 降安全等级要显式标志。主账号只放官方登录，不再有切换与保存实际配置。
        ("agents", "configure") => {
            specs.push(("--only", ArgValueKind::ElementId));
            specs.push(("--base", ArgValueKind::Digest));
            specs.push(("--preview-file-base", ArgValueKind::RuleBase));
            specs.push(("--runtime", ArgValueKind::JsonValue));
            specs.push(("--allow-yolo", ArgValueKind::Flag));
        }
        ("agents", "preview-runtime") => {
            specs.push(("--only", ArgValueKind::ElementId));
            specs.push(("--base", ArgValueKind::Digest));
            specs.push(("--runtime", ArgValueKind::JsonValue));
        }
        ("skills", "versions") => {
            specs.push(("--only", ArgValueKind::ElementId));
        }
        ("skills", "preview") => {
            specs.push(("--only", ArgValueKind::ElementId));
            specs.push(("--ref", ArgValueKind::GitRef));
        }
        ("skills", "adopt-preview") => {
            specs.push(("--name", ArgValueKind::ElementId));
            specs.push(("--collection", ArgValueKind::ElementId));
            specs.push(("--dest", ArgValueKind::FilePath));
        }
        ("skills", "adopt") => {
            specs.push(("--name", ArgValueKind::ElementId));
            specs.push(("--collection", ArgValueKind::ElementId));
            specs.push(("--dest", ArgValueKind::FilePath));
            specs.push(("--digest", ArgValueKind::Digest));
            specs.push(("--ref", ArgValueKind::GitRef));
            // 技能在来源仓库里的相对位置：与版本名同一套字符规则
            specs.push(("--path", ArgValueKind::GitRef));
            specs.push(("--keep-local", ArgValueKind::Flag));
            specs.push(("--accept-source", ArgValueKind::Flag));
            specs.push(("--undo", ArgValueKind::RunId));
            // 纳入时要链接的账号（逗号分隔；默认全部账号）
            specs.push(("--agents", ArgValueKind::AgentList));
        }
        (_, "apply") | ("mcp", "preview") => {
            specs.push(("--only", ArgValueKind::ElementId));
            if domain == "skills" {
                specs.push(("--agent", ArgValueKind::ElementId));
                specs.push(("--action", ArgValueKind::SkillAction));
                // 孤立链接清理：按名指定，不属于任何集合
                specs.push(("--link", ArgValueKind::ElementId));
                // 逐 skill add/remove 的显式接管通道（引擎侧再强制动作范围与非受管判定）
                specs.push(("--allow-takeover", ArgValueKind::Flag));
                // 移除集合时一并清理下载的仓库与没人再用的快照
                specs.push(("--purge", ArgValueKind::Flag));
                // 更新 git 来源到指定版本（切换 / 回滚）
                specs.push(("--ref", ArgValueKind::GitRef));
                specs.push(("--replace-from", ArgValueKind::ElementId));
                specs.push(("--replace-version", ArgValueKind::GitRef));
            }
            // MCP 逐条写入：条目内容（--spec，JSON）、写入基准摘要（--base，
            // 复制覆盖同名时另带目标的 --to-base）、复制目标助手（--to）。
            if domain == "mcp" {
                specs.push(("--agent", ArgValueKind::ElementId));
                specs.push(("--action", ArgValueKind::McpAction));
                specs.push(("--spec", ArgValueKind::JsonValue));
                specs.push(("--base", ArgValueKind::Digest));
                specs.push(("--to", ArgValueKind::ElementId));
                specs.push(("--to-base", ArgValueKind::Digest));
                specs.push(("--replace", ArgValueKind::Flag));
            }
            // 实例配置以实际文件为准：写入带 status 给出的基准，外部改过就被拒；「接管」已退役
            if domain == "profiles" {
                specs.push(("--base", ArgValueKind::RuleBase));
                specs.push(("--preview-file-base", ArgValueKind::RuleBase));
                specs.push(("--preview-content-base", ArgValueKind::Digest));
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
        ArgValueKind::LaunchShell => {
            if ["zsh", "bash", "powershell"].contains(&value) {Ok(())}
            else {Err(EngineError::new("invalid_args", "Unsupported launch shell"))}
        }
        ArgValueKind::LaunchName => {
            if value.len() <= 80 && value.chars().next().is_some_and(|c|c.is_ascii_alphabetic() || c=='_')
                && value.chars().all(|c|c.is_ascii_alphanumeric() || matches!(c,'_'|'-')) {Ok(())}
            else {Err(EngineError::new("invalid_args", "Invalid launch shortcut name"))}
        }
        ArgValueKind::AccountShell => {
            if ["posix", "powershell"].contains(&value) {
                Ok(())
            } else {
                Err(EngineError::new("invalid_args", "非法的账号终端类型"))
            }
        }
        ArgValueKind::AccountOperation => {
            if ["launch", "login", "logout", "status"].contains(&value) {
                Ok(())
            } else {
                Err(EngineError::new("invalid_args", "非法的账号操作"))
            }
        }
        ArgValueKind::Flag => Ok(()),
        ArgValueKind::ElementId => validate_element_id(value),
        ArgValueKind::SeedItems => {
            const ITEMS: &[&str] = &["permissions", "statusline", "rules", "mcp"];
            if !value.is_empty() && value.split(',').all(|item| ITEMS.contains(&item)) {
                Ok(())
            } else {
                Err(EngineError::new(
                    "invalid_args",
                    format!("非法的带入项 '{value}'（只能从 {} 里选）", ITEMS.join("、")),
                ))
            }
        }
        ArgValueKind::AgentList => {
            let ok = !value.is_empty()
                && value.split(',').all(|agent| {
                    !agent.is_empty()
                        && agent
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                });
            if ok {
                Ok(())
            } else {
                Err(EngineError::new(
                    "invalid_args",
                    format!("非法的账号列表 '{value}'"),
                ))
            }
        }
        ArgValueKind::GitRef => {
            let ok = !value.is_empty()
                && value.len() <= 200
                && !value.starts_with('-')
                && !value.contains("..")
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '+'));
            if ok {
                Ok(())
            } else {
                Err(EngineError::new(
                    "invalid_args",
                    format!("非法的版本 '{value}'"),
                ))
            }
        }
        ArgValueKind::RunId => validate_run_id(value),
        ArgValueKind::ProgramAction => {
            const ACTIONS: &[&str] = &[
                "install", "upgrade", "switch", "restore", "remove", "adopt", "unadopt", "link",
                "unlink",
            ];
            if ACTIONS.contains(&value) {
                Ok(())
            } else {
                Err(EngineError::new(
                    "invalid_args",
                    format!("非法程序动作 '{value}'"),
                ))
            }
        }
        ArgValueKind::BoolText => {
            if value == "true" || value == "false" {
                Ok(())
            } else {
                Err(EngineError::new(
                    "invalid_args",
                    format!("只能是 true 或 false，不是 '{value}'"),
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
        ArgValueKind::VarName => {
            let mut chars = value.chars();
            let ok = match chars.next() {
                Some(first) => {
                    (first.is_ascii_alphabetic() || first == '_')
                        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
                }
                None => false,
            };
            if ok {
                Ok(())
            } else {
                Err(EngineError::new(
                    "invalid_args",
                    format!("非法变量名 '{value}'（须以字母或下划线开头，只含字母、数字与下划线）"),
                ))
            }
        }
        // 取值本身不回显：任何拒绝消息都只说形状，不带值。
        ArgValueKind::SecretValue => {
            if value.chars().any(|c| c.is_control()) {
                Err(EngineError::new(
                    "invalid_args",
                    "凭据取值不能包含控制字符".to_string(),
                ))
            } else {
                Ok(())
            }
        }
        ArgValueKind::FilePath => {
            if value.len() > 4096 || value.chars().any(|c| c.is_control()) {
                Err(EngineError::new(
                    "invalid_args",
                    "外部密钥文件路径不合法".to_string(),
                ))
            } else {
                Ok(())
            }
        }
        ArgValueKind::Digest => {
            let ok = value.len() == 64
                && value
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
            if ok {
                Ok(())
            } else {
                Err(EngineError::new(
                    "invalid_args",
                    "写入基准必须是 64 位小写十六进制摘要".to_string(),
                ))
            }
        }
        ArgValueKind::RuleText => Ok(()),
        ArgValueKind::RuleBase => {
            if value == "missing" || is_sha256_hex(value) {
                Ok(())
            } else {
                Err(EngineError::new(
                    "invalid_args",
                    "--base 必须是 64 位小写 sha256 或 missing",
                ))
            }
        }
        ArgValueKind::SuggestionId => {
            let ok = ["cleanup-", "legacy-"].iter().any(|prefix| {
                value.strip_prefix(prefix).is_some_and(|hex| {
                    hex.len() == 16
                        && hex
                            .chars()
                            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
                })
            });
            if ok {
                Ok(())
            } else {
                Err(EngineError::new("invalid_args", "非法 Hooks 建议 id"))
            }
        }
        ArgValueKind::Revision => {
            if is_sha256_hex(value) {
                Ok(())
            } else {
                Err(EngineError::new(
                    "invalid_args",
                    "--revision 必须是 64 位小写 sha256",
                ))
            }
        }
        ArgValueKind::TemplateId => {
            let ok = !value.is_empty()
                && value.len() <= 64
                && value
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                && value
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if ok {
                Ok(())
            } else {
                Err(EngineError::new("invalid_args", "非法规则模板 id"))
            }
        }
        ArgValueKind::SourceRuleId => {
            let part = |p: &str| {
                !p.is_empty()
                    && p != "."
                    && p != ".."
                    && p.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            };
            let ok = value.len() <= 200
                && matches!(value.split_once('/'), Some((coll, name)) if part(coll) && part(name));
            if ok {
                Ok(())
            } else {
                Err(EngineError::new(
                    "invalid_args",
                    "来源规则 id 须为 <集合>/<名字>",
                ))
            }
        }
        // 名称不回显：只说形状
        ArgValueKind::TemplateName => {
            if value.len() > 400 || value.chars().any(|c| c.is_control()) {
                Err(EngineError::new(
                    "invalid_args",
                    "规则模板名称不能包含换行或控制字符，且不能过长",
                ))
            } else {
                Ok(())
            }
        }
        ArgValueKind::JsonValue => serde_json::from_str::<serde_json::Value>(value)
            .map(|_| ())
            .map_err(|err| {
                EngineError::new("invalid_args", format!("--value 不是合法 JSON: {err}"))
            }),
        ArgValueKind::SkillAction => match value {
            "add" | "update" | "enable" | "disable" | "remove" | "remove-link" => Ok(()),
            _ => Err(EngineError::new("invalid_args", "非法 skills action")),
        },
        ArgValueKind::McpAction => match value {
            "add" | "edit" | "clean-markers" | "enable" | "disable" | "remove" | "copy" | "forget" => Ok(()),
            _ => Err(EngineError::new(
                "invalid_args",
                format!(
                    "非法 mcp action '{value}'（仅 add|edit|clean-markers|enable|disable|remove|copy|forget）"
                ),
            )),
        },
    }
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// 规则目标 id：两个全局目标，或 `rules:profile:<实例 id>`（实例 id 与别名同一规则）。
fn is_rules_target(value: &str) -> bool {
    if value == "rules:codex_global" || value == "rules:claude_global" {
        return true;
    }
    value.strip_prefix("rules:profile:").is_some_and(|id| {
        id.chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    })
}

pub fn validate_args(command: &str, args: &[String]) -> Result<(), EngineError> {
    let specs = allowed_arg_specs(command);
    let (domain, _) = command.split_once(':').unwrap_or(("", ""));
    let is_rules = domain == "rules";
    // rules 与 aliases 同 native 严格参数保持一致：拒绝重复 flag。
    let reject_duplicates = is_rules || domain == "aliases" || domain == "launch";
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
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
        if *kind != ArgValueKind::Flag {
            index += 1;
            let Some(value) = args.get(index) else {
                return Err(EngineError::new(
                    "invalid_args",
                    format!("参数 '{arg}' 缺少取值"),
                ));
            };
            validate_arg_value(*kind, value)?;
            if is_rules && (arg == "--only" || arg == "--from") && !is_rules_target(value) {
                return Err(EngineError::new(
                    "invalid_args",
                    format!(
                        "非法 rules 目标 '{value}'（仅 rules:codex_global|rules:claude_global|rules:profile:<实例>）"
                    ),
                ));
            }
        }
        index += 1;
    }
    Ok(())
}

fn dispatch_native(
    ctx: agent_manager_engine::Context,
    command: String,
    args: Vec<String>,
    run_id: String,
) -> Result<agent_manager_engine::Envelope, EngineError> {
    validate_command(&command)?;
    validate_args(&command, &args)?;
    validate_run_id(&run_id)?;
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
    let home_dir = agent_manager_engine::platform::home_dir().ok_or_else(|| {
        EngineError::new(
            "home_unavailable",
            "找不到用户主目录（HOME / USERPROFILE 都没设置）",
        )
    })?;
    tauri::async_runtime::spawn_blocking(move || {
        dispatch_native(
            agent_manager_engine::Context { data_dir, home_dir },
            command,
            args.unwrap_or_default(),
            run_id.unwrap_or_else(|| format!("tauri-{}", std::process::id())),
        )
    })
    .await
    .map_err(|e| EngineError::new("execution_failed", e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_generation_is_exposed_without_retired_account_commands() {
        for command in ["launch:list", "launch:save", "launch:delete"] {
            assert!(validate_command(command).is_ok());
        }
        assert!(validate_args("launch:save", &["--spec".into(), "{}".into()]).is_ok());
        assert!(validate_args("launch:save", &["--confirm".into()]).is_err());
        assert!(validate_command("launch:build").is_ok());
        assert!(validate_command("launch:catalog").is_ok());
        assert!(validate_args("launch:catalog", &[]).is_ok());
        assert!(validate_args("launch:catalog", &["--confirm".into()]).is_err());
        assert!(validate_command("launch:run").is_ok());
        assert!(validate_args(
            "launch:run",
            &[
                "--spec".into(),
                "{}".into(),
                "--name".into(),
                "example-sg".into(),
                "--confirm".into()
            ]
        )
        .is_ok());
        assert!(validate_args("launch:build", &["--spec".into(), "{}".into()]).is_ok());
        assert!(validate_args("launch:build", &["--secret-value".into(), "value".into()]).is_err());
        assert!(validate_command("aliases:apply").is_err());
        assert!(validate_command("accounts:launch").is_err());
    }

    #[test]
    fn controlled_account_commands_reject_unsupported_operations_and_secret_flags() {
        assert!(validate_command("accounts:command").is_err());
        assert!(validate_command("accounts:launch").is_err());
        assert!(validate_command("profiles:create").is_err());
        let args = |items: &[&str]| {
            items
                .iter()
                .map(|item| (*item).to_string())
                .collect::<Vec<_>>()
        };
        assert!(validate_args(
            "accounts:command",
            &args(&[
                "--only",
                "profiles:work",
                "--operation",
                "logout",
                "--shell",
                "powershell"
            ])
        )
        .is_ok());
        assert!(validate_args("accounts:command", &args(&["--shell", "cmd"])).is_err());
        assert!(validate_args("accounts:command", &args(&["--operation", "shell"])).is_err());
        assert!(validate_args(
            "accounts:command",
            &args(&["--secret-value", "YOUR_TEST_KEY"])
        )
        .is_err());
        let digest = "a".repeat(64);
        let create = args(&[
            "--only",
            "profiles:work",
            "--definition",
            "{}",
            "--base",
            "missing",
            "--preview-file-base",
            "missing",
            "--preview-content-base",
            &digest,
            "--secret-var",
            "AM_WORK_KEY",
            "--secret-value",
            "YOUR_TEST_KEY",
        ]);
        assert!(validate_args("profiles:create", &create).is_ok());
        assert!(validate_args(
            "profiles:create",
            &args(&["--secret-value", "YOUR_TEST_KEY\n"])
        )
        .is_err());
        assert!(validate_args("profiles:create", &args(&["--allow-takeover"])).is_err());
    }

    #[test]
    fn command_whitelist_covers_contract_surface_and_rejects_injection() {
        for command in [
            "mcp:status",
            "mcp:preview",
            "mcp:check",
            "mcp:apply",
            "skills:status",
            "skills:apply",
            "rules:status",
            "rules:apply",
            "rules:save-template",
            "rules:delete-template",
            "secrets:status",
            "secrets:set",
            "secrets:source",
            "system:audit",
            "system:discover",
            "system:refresh-upstream",
            "runs:list",
            "runs:get",
            "runs:clear",
            "policy:get",
            "policy:set",
            "policy:delete",
            "db:status",
            "db:migrate",
            "config:export",
            "config:import",
            "resources:targets",
        ] {
            assert!(validate_command(command).is_ok(), "{command} should pass");
        }

        for command in [
            "agents:status",
            "agents:configure",
            "agents:preview-runtime",
            "agents:auth-status",
            "aliases:status",
            "aliases:preview",
            "aliases:apply",
            "aliases:discard-legacy",
            "profiles:status",
            "profiles:preview",
            "profiles:apply",
            "profiles:apply-definition",
            "profiles:adopt",
            "profiles:seed",
            "profiles:shared-status",
            "profiles:shared-apply",
            "secrets:migrate-legacy",
            "usage:status",
            "usage:scan",
            "usage:clear",
            "usage:sessions",
            "agents:apply",
            "agents:update",
            "agents:plan",
            "mcp:plan",
            "skills:plan",
            "rules:plan",
            "rules:templates",
            "rules:set",
            "hooks:save-template",
            "hooks:plan",
            // 2026-09-29 Hooks 功能移除
            "hooks:status",
            "hooks:preview",
            "hooks:apply",
            "hooks:discard-legacy",
            "aliases:plan",
            "secrets:plan",
            "secrets:apply",
            "system:status",
            // 2026-09-28 多账号设计退役：主配置切换、接入检查 / 模型 / 余额、配置原文编辑
            "agents:preview-default",
            "agents:check",
            "agents:models",
            "agents:balance",
            "agents:config-file",
            "agents:edit-config",
            "profiles:config-file",
            "profiles:shared-preview",
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
            // 别名来源已退役：sources 整个域不再在命令面内
            "sources:get",
            "sources:set",
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
        for action in ["add", "update", "enable", "disable", "remove"] {
            assert!(validate_args(
                "skills:apply",
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
                    action.into(),
                ]
            )
            .is_ok());
        }

        for (command, args) in [
            (
                "system:refresh-upstream",
                vec!["--only".into(), "mcp:example-search".into()],
            ),
            ("system:discover", vec!["--only".into(), "skills".into()]),
            ("config:export", vec![]),
            (
                "secrets:source",
                vec!["--path".into(), "~/private/secrets.env".into()],
            ),
            ("secrets:source", vec!["--path".into(), "".into()]),
            (
                "secrets:source",
                vec![
                    "--path".into(),
                    "~/.config/agent-manager/secrets.env".into(),
                    "--create".into(),
                ],
            ),
            ("secrets:migrate-legacy", vec![]),
            (
                "config:import",
                vec!["--value".into(), "{\"format\":\"x\"}".into()],
            ),
            (
                "skills:apply",
                vec![
                    "--action".into(),
                    "remove-link".into(),
                    "--agent".into(),
                    "claude-code".into(),
                    "--link".into(),
                    "stale-one".into(),
                ],
            ),
            (
                "mcp:apply",
                vec![
                    "--only".into(),
                    "mcp:example-search".into(),
                    "--action".into(),
                    "disable".into(),
                ],
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
                ],
            ),
            (
                "policy:set",
                vec![
                    "--layer".into(),
                    "local".into(),
                    "--key".into(),
                    "agents.definitions.fixture".into(),
                    "--value".into(),
                    "{}".into(),
                    "--expect-definition".into(),
                    "{}".into(),
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
        ] {
            assert!(
                validate_args(command, &args).is_ok(),
                "{command} {args:?} should pass"
            );
        }

        for (command, args) in [
            // plan 命令已被命令面拒绝，参数校验不会到达此处；--dry-run 已从白名单移除。
            ("mcp:status", vec!["--dry-run".into()]),
            ("mcp:apply", vec!["--dry-run".into()]),
            ("rules:apply", vec!["--dry-run".into()]),
            ("policy:set", vec!["--dry-run".into()]),
            ("sources:set", vec!["--dry-run".into()]),
            ("secrets:source", vec!["--path".into(), "bad\npath".into()]),
            ("secrets:migrate-legacy", vec!["--force".into()]),
            ("mcp:apply", vec!["--allow-uninstall".into()]),
            ("agents:status", vec!["--action".into(), "install".into()]),
            ("skills:apply", vec!["--action".into(), "install".into()]),
            ("runs:list", vec!["--run".into(), "r1".into()]),
            ("policy:get", vec!["--dry-run".into()]),
            ("sources:set", vec!["--def-file".into(), "elsewhere".into()]),
            ("system:refresh-upstream", vec!["--only".into()]),
            ("system:discover", vec!["--dry-run".into()]),
            ("config:export", vec!["--value".into(), "{}".into()]),
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
            ("skills:apply", vec!["--action".into(), "INSTALL".into()]),
            ("mcp:status", vec!["; rm -rf /".into()]),
            ("mcp:status", vec!["$(reboot)".into()]),
            ("mcp:status", vec!["mcp:status --offline".into()]),
        ] {
            assert!(
                validate_args(command, &args).is_err(),
                "{command} {args:?} should fail"
            );
        }

        let digest = "a".repeat(64);
        for action in [
            "add",
            "edit",
            "clean-markers",
            "enable",
            "disable",
            "remove",
            "copy",
            "forget",
        ] {
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
        assert!(validate_args(
            "mcp:check",
            &[
                "--only".into(),
                "mcp:s1".into(),
                "--agent".into(),
                "codex".into(),
                "--base".into(),
                digest.clone(),
                "--cwd".into(),
                "/isolated/project with spaces".into(),
            ]
        )
        .is_ok());
        // 逐条写入的完整参数面：--spec JSON、--base/--to-base 摘要、--to、--replace
        assert!(validate_args(
            "mcp:apply",
            &[
                "--only".into(),
                "mcp:s1".into(),
                "--agent".into(),
                "claude-code".into(),
                "--to".into(),
                "codex".into(),
                "--action".into(),
                "copy".into(),
                "--base".into(),
                digest.clone(),
                "--replace".into(),
                "--to-base".into(),
                digest.clone(),
            ]
        )
        .is_ok());
        assert!(validate_args(
            "mcp:preview",
            &[
                "--only".into(),
                "mcp:s1".into(),
                "--agent".into(),
                "codex".into(),
                "--action".into(),
                "edit".into(),
                "--base".into(),
                digest.clone(),
                "--spec".into(),
                r#"{"command":"npx","args":[]}"#.into(),
            ]
        )
        .is_ok());
        assert!(validate_args(
            "mcp:apply",
            &[
                "--only".into(),
                "mcp:s1".into(),
                "--agent".into(),
                "codex".into(),
                "--action".into(),
                "edit".into(),
                "--base".into(),
                digest.clone(),
                "--spec".into(),
                r#"{"command":"npx","args":["-y","pkg"],"env":{"K":"<literal>"}}"#.into(),
            ]
        )
        .is_ok());
        for bad in [
            vec!["--base".to_string(), "ABC".to_string()],
            vec!["--base".to_string(), "A".repeat(64)],
            vec!["--spec".to_string(), "{not json".to_string()],
        ] {
            let mut args: Vec<String> = vec!["--only".into(), "mcp:s1".into()];
            args.extend(bad.clone());
            assert!(
                validate_args("mcp:apply", &args).is_err(),
                "{bad:?} should fail"
            );
        }
        // 已退役：按库版本升级、update、显式接管、导入时的密钥接管命令
        for action in ["update", "upgrade", "install"] {
            let err = validate_args(
                "mcp:apply",
                &[
                    "--only".into(),
                    "mcp:s1".into(),
                    "--action".into(),
                    action.into(),
                ],
            )
            .unwrap_err();
            assert!(
                err.message.contains("非法 mcp action"),
                "MCP action error must mention mcp action, got: {}",
                err.message
            );
            assert!(!err.message.contains("skills"));
        }
        assert!(
            validate_args(
                "mcp:apply",
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
            .is_err(),
            "mcp:apply 已不再接受 --allow-takeover"
        );
        assert!(validate_command("secrets:adopt").is_err());
        // aliases 以 rc 文件受管区块的实际行为准：按 base 直接编辑，「接管」已退役
        assert!(
            validate_args(
                "aliases:apply",
                &[
                    "--only".into(),
                    "aliases:zshrc".into(),
                    "--allow-takeover".into()
                ]
            )
            .is_err(),
            "aliases:apply --allow-takeover should fail"
        );
        // skills:apply 放行（逐 skill add/remove 显式接管，引擎侧再强制动作范围）
        assert!(
            validate_args(
                "skills:apply",
                &[
                    "--only".into(),
                    "skill:repo/name".into(),
                    "--agent".into(),
                    "codex".into(),
                    "--action".into(),
                    "add".into(),
                    "--allow-takeover".into(),
                ]
            )
            .is_ok(),
            "skills:apply --allow-takeover should pass"
        );
        for (command, args) in [
            ("mcp:status", vec!["--allow-takeover".into()]),
            ("skills:status", vec!["--allow-takeover".into()]),
            ("rules:status", vec!["--allow-takeover".into()]),
            // 规则以实际文件为准，没有「接管」：直接编辑本身就是写实际文件
            (
                "rules:apply",
                vec![
                    "--only".into(),
                    "rules:claude_global".into(),
                    "--allow-takeover".into(),
                ],
            ),
            ("aliases:status", vec!["--allow-takeover".into()]),
        ] {
            assert!(
                validate_args(command, &args).is_err(),
                "{command} --allow-takeover should fail"
            );
        }

        // Rules 契约：写一个目标（--only + --base + 四种来源之一）与模板保存/删除。
        let base = "a".repeat(64);
        let only = |target: &str| vec!["--only".to_string(), target.to_string()];
        let with = |target: &str, extra: &[&str]| {
            let mut args = only(target);
            args.extend(extra.iter().map(|s| s.to_string()));
            args
        };
        for (command, args) in [
            (
                "rules:apply",
                with(
                    "rules:claude_global",
                    &[
                        "--base",
                        &base,
                        "--value",
                        "---\nfront: matter\n---\n正文\n",
                    ],
                ),
            ),
            (
                "rules:apply",
                with(
                    "rules:codex_global",
                    &["--base", "missing", "--source-rule", "agent-skills/core"],
                ),
            ),
            (
                "rules:apply",
                with(
                    "rules:profile:cc-ly",
                    &["--base", "missing", "--template", "tpl-0a1b"],
                ),
            ),
            (
                "rules:apply",
                with(
                    "rules:codex_global",
                    &["--base", &base, "--from", "rules:profile:cx-work"],
                ),
            ),
            (
                "rules:save-template",
                vec![
                    "--name".into(),
                    "团队规则".into(),
                    "--value".into(),
                    "多行\n正文\n".into(),
                ],
            ),
            (
                "rules:save-template",
                vec![
                    "--template".into(),
                    "legacy-claude".into(),
                    "--revision".into(),
                    base.clone(),
                    "--name".into(),
                    "改名".into(),
                    "--value".into(),
                    "".into(),
                ],
            ),
            (
                "rules:delete-template",
                vec!["--template".into(), "tpl-0a1b".into()],
            ),
        ] {
            assert!(
                validate_args(command, &args).is_ok(),
                "{command} {args:?} should pass"
            );
        }

        // Rules 拒绝：status 任意 raw arg（含 --offline，只能由 bridge 注入）、
        // 重复 flag、未知/别名 target、非法 base/模板 id/名称、缺值、退役的 --dry-run。
        for (command, args) in [
            ("rules:status", vec!["--offline".into()]),
            ("rules:status", only("rules:codex_global")),
            ("rules:status", vec!["--dry-run".into()]),
            ("rules:status", vec!["positional".into()]),
            ("rules:apply", vec!["--offline".into()]),
            (
                "rules:apply",
                with("rules:codex_global", &["--only", "rules:claude_global"]),
            ),
            (
                "rules:apply",
                with(
                    "rules:codex_global",
                    &["--base", "missing", "--base", "missing"],
                ),
            ),
            ("rules:apply", only("rules:unknown")),
            ("rules:apply", only("rules:codex")),
            ("rules:apply", only("codex")),
            ("rules:apply", only("rules:profile:Bad_Id")),
            ("rules:apply", vec!["--only".into()]),
            ("rules:apply", vec!["--dry-run".into()]),
            (
                "rules:apply",
                with("rules:codex_global", &["--base", "not-a-digest"]),
            ),
            (
                "rules:apply",
                with("rules:codex_global", &["--from", "codex"]),
            ),
            (
                "rules:apply",
                with("rules:codex_global", &["--template", "Bad Id"]),
            ),
            // 内置推荐已退役；来源规则 id 形状不对的拒绝
            (
                "rules:apply",
                with(
                    "rules:codex_global",
                    &["--base", "missing", "--recommended"],
                ),
            ),
            (
                "rules:apply",
                with("rules:codex_global", &["--source-rule", "../core"]),
            ),
            (
                "rules:apply",
                with("rules:codex_global", &["--source-rule", "a/b/c"]),
            ),
            (
                "rules:apply",
                with("rules:codex_global", &["--source-rule", "core"]),
            ),
            (
                "rules:save-template",
                vec!["--name".into(), "两\n行".into()],
            ),
            (
                "rules:save-template",
                vec!["--revision".into(), "short".into()],
            ),
            ("rules:save-template", vec!["--recommended".into()]),
            ("rules:delete-template", vec!["--template".into()]),
            (
                "rules:delete-template",
                vec!["--template".into(), "../x".into()],
            ),
        ] {
            assert!(
                validate_args(command, &args).is_err(),
                "{command} {args:?} should fail"
            );
        }
    }

    #[test]
    fn agents_runtime_and_auth_args_follow_the_contract() {
        let base = "c".repeat(64);
        let args = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(validate_command("agents:preview-runtime").is_err());
        assert!(validate_args(
            "agents:configure",
            &args(&[
                "--only",
                "agents:claude-code",
                "--base",
                &base,
                "--runtime",
                r#"{"statusline":"builtin"}"#,
                "--preview-file-base",
                &base
            ])
        )
        .is_ok());
        assert!(validate_args("agents:auth-status", &args(&["--only", "profiles:cc-ly"])).is_ok());
        assert!(validate_args(
            "agents:auth-status",
            &args(&["--only", "profiles:cc-ly", "--probe"])
        )
        .is_ok());
        assert!(validate_command("skills:fork").is_ok());
        assert!(validate_args(
            "skills:fork",
            &args(&[
                "--only",
                "skill:agent-skills/verification",
                "--dest",
                "~/my-skills",
                "--collection",
                "my-skills"
            ])
        )
        .is_ok());
        assert!(validate_args(
            "skills:fork",
            &args(&["--only", "skill:a/b", "--action", "remove"])
        )
        .is_err());
        assert!(validate_args(
            "agents:auth-status",
            &args(&["--only", "agents:claude-code", "--base", &base])
        )
        .is_err());
    }

    /// 实例以实际文件为准：写入与采纳都带基准；「接管」参数已退役。
    #[test]
    fn profiles_args_follow_the_actual_config_contract() {
        let base = "b".repeat(64);
        let args = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(validate_command("profiles:adopt").is_err());
        assert!(validate_command("profiles:preview").is_err());
        assert!(validate_args(
            "profiles:preview",
            &args(&[
                "--only",
                "profiles:cc-ly",
                "--definition",
                "{\"profile\":{}}"
            ])
        )
        .is_ok());
        assert!(validate_args(
            "profiles:preview",
            &args(&[
                "--only",
                "profiles:cc-ly",
                "--definition",
                "{\"profile\":{}}",
                "--on-save",
                "--items",
                "permissions,statusline,rules,mcp"
            ])
        )
        .is_ok());
        assert!(validate_args(
            "profiles:preview",
            &args(&[
                "--only",
                "profiles:cc-ly",
                "--definition",
                "{\"profile\":{}}",
                "--items",
                "permissions,statusline,rules,mcp"
            ])
        )
        .is_ok());
        assert!(validate_args(
            "profiles:preview",
            &args(&[
                "--only",
                "profiles:cc-ly",
                "--definition",
                "{\"profile\":{}}",
                "--items",
                "env"
            ])
        )
        .is_err());
        assert!(validate_args(
            "profiles:preview",
            &args(&["--only", "profiles:cc-ly", "--definition", "not json"])
        )
        .is_err());
        assert!(validate_args(
            "profiles:preview",
            &args(&[
                "--only",
                "profiles:cc-ly",
                "--base",
                &base,
                "--definition-base",
                &base,
                "--candidate",
                "{\"source_connection\":\"relay\"}"
            ])
        )
        .is_ok());
        assert!(validate_args(
            "profiles:preview",
            &args(&[
                "--only",
                "profiles:cc-ly",
                "--base",
                &base,
                "--definition-base",
                &base,
                "--candidate",
                "{}",
                "--exact",
                "--preview-secret-var",
                "RELAY_KEY",
                "--preview-secret-value",
                "draft-key",
            ])
        )
        .is_ok());
        assert!(validate_args(
            "profiles:preview",
            &args(&[
                "--preview-secret-var",
                "RELAY_KEY",
                "--preview-secret-value",
                "bad\nkey"
            ])
        )
        .is_err());
        for command in ["profiles:apply", "profiles:adopt"] {
            assert!(
                validate_args(
                    command,
                    &args(&["--only", "profiles:cc-ly", "--base", &base])
                )
                .is_ok(),
                "{command} 带 base 应放行"
            );
            assert!(validate_args(
                command,
                &args(&["--only", "profiles:cc-ly", "--base", "missing"])
            )
            .is_ok());
            assert!(validate_args(
                command,
                &args(&["--only", "profiles:cc-ly", "--base", "XYZ"])
            )
            .is_err());
        }
        assert!(validate_args(
            "profiles:preview",
            &args(&["--only", "profiles:cc-ly", "--base", &base])
        )
        .is_ok());
        assert!(validate_args(
            "profiles:apply",
            &args(&[
                "--only",
                "profiles:cc-ly",
                "--base",
                &base,
                "--preview-file-base",
                "missing",
                "--preview-content-base",
                &base
            ])
        )
        .is_ok());
        assert!(validate_args(
            "profiles:apply",
            &args(&[
                "--only",
                "profiles:cc-ly",
                "--base",
                &base,
                "--preview-content-base",
                "missing"
            ])
        )
        .is_err());
        assert!(validate_args(
            "profiles:adopt",
            &args(&[
                "--only",
                "profiles:cc-ly",
                "--base",
                &base,
                "--preview-file-base",
                "missing"
            ])
        )
        .is_err());
        assert!(validate_args(
            "profiles:apply",
            &args(&["--only", "profiles:cc-ly", "--allow-takeover"])
        )
        .is_err());
        let candidate = r#"{"source_connection":"relay","model":"new-model"}"#;
        assert!(validate_args(
            "profiles:apply-definition",
            &args(&[
                "--only",
                "profiles:cc-ly",
                "--base",
                "missing",
                "--definition-base",
                &base,
                "--candidate",
                candidate,
                "--preview-file-base",
                "missing",
                "--preview-content-base",
                &base,
            ])
        )
        .is_ok());
        assert!(validate_args(
            "profiles:apply-definition",
            &args(&[
                "--only",
                "profiles:cc-ly",
                "--base",
                "missing",
                "--definition-base",
                "XYZ",
                "--candidate",
                candidate,
            ])
        )
        .is_err());
        // seed：只认那几项，逗号分隔；凭据相关的 env / apiKeyHelper 不在其中
        assert!(validate_args(
            "profiles:seed",
            &args(&[
                "--only",
                "profiles:cc-ly",
                "--items",
                "permissions,rules,mcp"
            ])
        )
        .is_ok());
        for items in [
            "",
            "env",
            "permissions,env",
            "mcp;rm",
            "permissions, rules",
            "permissions,hooks",
        ] {
            assert!(
                validate_args(
                    "profiles:seed",
                    &args(&["--only", "profiles:cc-ly", "--items", items])
                )
                .is_err(),
                "--items {items:?} 应拒绝"
            );
        }
        assert!(validate_args(
            "profiles:seed",
            &args(&[
                "--only",
                "profiles:cc-ly",
                "--items",
                "mcp",
                "--base",
                &base
            ])
        )
        .is_err());
        assert!(validate_args(
            "profiles:shared-status",
            &args(&["--only", "profiles:cc-ly"])
        )
        .is_ok());
        assert!(validate_command("profiles:shared-preview").is_err());
        assert!(validate_args(
            "profiles:shared-apply",
            &args(&[
                "--only",
                "profiles:cc-ly",
                "--item",
                "mcp:fs",
                "--source-base",
                &base,
                "--target-base",
                "missing"
            ])
        )
        .is_ok());
        assert!(validate_args(
            "profiles:shared-apply",
            &args(&[
                "--only",
                "profiles:cc-ly",
                "--item",
                "permissions",
                "--source-base",
                "stale",
                "--target-base",
                "missing"
            ])
        )
        .is_err());
    }

    #[test]
    fn agents_configure_args_require_digest_shape_and_reject_retired_takeover() {
        let digest = "a".repeat(64);
        for extra in [vec![
            "--runtime".to_string(),
            r#"{"automation_mode":"full"}"#.into(),
            "--allow-yolo".into(),
        ]] {
            let mut args = vec![
                "--only".to_string(),
                "agents:claude-code".into(),
                "--base".into(),
                digest.clone(),
            ];
            args.extend(extra);
            assert!(validate_args("agents:configure", &args).is_ok(), "{args:?}");
        }
        for (flag, value) in [
            ("--base", "ABC"),
            ("--base", "g".repeat(64).as_str()),
            ("--runtime", "{not json"),
            // 主账号只放官方登录：写入第三方、切回官方、保存实际配置在桥上就不放行
            ("--use", "relay"),
            ("--connection", r#"{"base_url":"https://x"}"#),
            ("--use-default", ""),
            ("--save-current", ""),
            ("--discard-live", ""),
        ] {
            assert!(
                validate_args(
                    "agents:configure",
                    &[
                        "--only".into(),
                        "agents:codex".into(),
                        flag.into(),
                        value.into()
                    ]
                )
                .is_err(),
                "{flag} {value} should fail"
            );
        }
        assert!(validate_args(
            "agents:configure",
            &[
                "--only".into(),
                "agents:codex".into(),
                "--base".into(),
                digest,
                "--use-default".into(),
                "--allow-takeover".into()
            ]
        )
        .is_err());
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
    fn system_audit_bridge_args_stay_strict_and_read_only() {
        // 手动离线模式已退役：界面不再注入，用户也传不进 --offline
        assert!(validate_args("system:audit", &["--offline".into()]).is_err());
        assert!(validate_args("system:audit", &[]).is_ok());
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
        let envelope =
            dispatch_native(ctx, "system:audit".into(), vec![], "audit-test".into()).unwrap();
        assert!(envelope.ok, "{:?}", envelope.errors);
        assert!(!envelope.mode.offline);
        // fresh 上下文没有错误；是否 warning 只取决于本机 PATH 下的助手探测
        //（终端快捷命令没有区块是正常空状态，不再算待生成）
        assert!(matches!(
            envelope.data["status"].as_str(),
            Some("warning" | "healthy")
        ));
        assert!(envelope.data["sections"].get("aliases").is_none());
        assert!(envelope.data["sections"].get("agents").is_none());
        assert_eq!(envelope.data["sections"].as_object().unwrap().len(), 6);
        assert!(!base.join("data").exists(), "audit 不得创建数据目录");
        assert!(!base.join("home").exists(), "audit 不得创建 home 目录");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn secrets_status_bridge_args_stay_strict_and_read_only() {
        assert!(validate_args("secrets:status", &["--offline".into()]).is_err());
        assert!(validate_args("secrets:status", &[]).is_ok());
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
        let envelope =
            dispatch_native(ctx, "secrets:status".into(), vec![], "secrets-test".into()).unwrap();
        assert!(envelope.ok, "{:?}", envelope.errors);
        assert!(!envelope.mode.offline);
        assert_eq!(envelope.data["domain"], "secrets");
        assert_eq!(envelope.data["elements"][0]["id"], "secrets:mcp-env");
        assert!(!base.join("data").exists(), "status 不得创建数据目录");
        assert!(!base.join("home").exists(), "status 不得创建 home 目录");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn aliases_args_follow_the_actual_block_contract() {
        let base = "a".repeat(64);
        let args = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for (command, list) in [
            (
                "aliases:apply",
                args(&[
                    "--zshrc-base",
                    &base,
                    "--zprofile-base",
                    "missing",
                    "--set",
                    "cc-work",
                    "--value",
                    "TZ=\"UTC\" claude --model opus",
                    "--rename-from",
                    "cc",
                ]),
            ),
            (
                "aliases:preview",
                args(&[
                    "--zshrc-base",
                    &base,
                    "--zprofile-base",
                    "missing",
                    "--set",
                    "cc-work",
                    "--value",
                    "CLAUDE_CONFIG_DIR=\"$HOME/.claude-cc-work\" claude",
                ]),
            ),
            (
                "aliases:apply",
                args(&["--zprofile-base", "missing", "--remove", "ll"]),
            ),
            // Windows：命令脚本目录的写入基准
            (
                "aliases:apply",
                args(&["--cmd-base", &base, "--remove", "ll"]),
            ),
            // bash 用户的 ~/.bashrc
            (
                "aliases:apply",
                args(&["--bashrc-base", &base, "--remove", "ll"]),
            ),
            (
                "aliases:apply",
                args(&["--zshrc-base", &base, "--recommended", "claude"]),
            ),
            (
                "aliases:apply",
                args(&["--zshrc-base", &base, "--legacy", "legacy-0123456789abcdef"]),
            ),
            (
                "aliases:discard-legacy",
                args(&["--legacy", "legacy-0123456789abcdef"]),
            ),
        ] {
            assert!(
                validate_args(command, &list).is_ok(),
                "{command} {list:?} should pass"
            );
        }
        for (command, list) in [
            ("aliases:status", args(&["--offline"])),
            ("aliases:apply", args(&["--only", "aliases:zshrc"])),
            ("aliases:apply", args(&["--zshrc-base", "nothex"])),
            ("aliases:preview", args(&["--zshrc-base", "nothex"])),
            ("aliases:apply", args(&["--cmd-base", "nothex"])),
            (
                "aliases:apply",
                args(&["--zshrc-base", "missing", "--zshrc-base", "missing"]),
            ),
            (
                "aliases:apply",
                args(&["--zshrc-base", "missing", "--remove", "bad name"]),
            ),
            (
                "aliases:apply",
                args(&["--zshrc-base", "missing", "--legacy", "legacy-xyz"]),
            ),
            (
                "aliases:apply",
                args(&["--zshrc-base", "missing", "--dry-run"]),
            ),
            ("aliases:discard-legacy", args(&["--legacy", "Bad"])),
            ("aliases:discard-legacy", args(&["--remove", "ll"])),
        ] {
            assert!(
                validate_args(command, &list).is_err(),
                "{command} {list:?} should fail"
            );
        }

        // 端到端：bridge 直连引擎，status 只读零写
        let root = std::env::temp_dir().join(format!("am-aliases-bridge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let result = dispatch_native(
            agent_manager_engine::Context {
                data_dir: root.join("data"),
                home_dir: root.join("home"),
            },
            "aliases:status".into(),
            vec![],
            "aliases-bridge".into(),
        );
        assert!(result.is_err());
        assert!(!root.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rules_status_bridge_args_stay_strict_and_read_only() {
        // 手动离线模式已退役：--offline 仍被 validate_args 拒绝，界面也不再注入。
        assert!(validate_args("rules:status", &["--offline".into()]).is_err());

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
        let env =
            dispatch_native(c(), "rules:status".into(), vec![], "rules-bridge".into()).unwrap();
        assert!(env.ok, "{:?}", env.errors);
        assert!(!env.mode.offline);
        // rules 固定两目标：codex、claude
        assert_eq!(env.data["elements"].as_array().unwrap().len(), 2);
        assert!(!root.join("data").exists(), "rules:status 零写");
        assert!(!root.join("home").exists(), "rules:status 零写");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dispatch_read_write_read() {
        let root = std::env::temp_dir().join(format!("am-dispatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let c = || agent_manager_engine::Context {
            data_dir: root.join("data"),
            home_dir: root.join("home"),
        };
        let targets =
            dispatch_native(c(), "resources:targets".into(), vec![], "labels".into()).unwrap();
        assert!(targets.ok);
        assert!(targets.data["targets"].is_array());
        for command in [
            "agents:status",
            "profiles:create",
            "accounts:command",
            "aliases:apply",
            "secrets:migrate-legacy",
        ] {
            assert!(dispatch_native(c(), command.into(), vec![], "retired".into()).is_err());
        }
        assert!(!root.exists());
        assert!(
            dispatch_native(c(), "policy:get".into(), vec![], "read".into())
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
                    "skills.x".into(),
                    "--value".into(),
                    "true".into()
                ],
                "write".into(),
            )
            .unwrap()
            .ok
        );
        assert_eq!(
            dispatch_native(c(), "policy:get".into(), vec![], "read".into())
                .unwrap()
                .data["effective"]["skills"]["x"],
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

    #[test]
    fn skills_versions_and_ref_are_allowed_and_validated() {
        assert!(validate_command("skills:versions").is_ok());
        assert!(validate_args(
            "skills:versions",
            &["--only".into(), "skill:archify".into()]
        )
        .is_ok());
        assert!(validate_args("skills:versions", &["--ref".into(), "v1".into()]).is_err());
        let update = |reference: &str| {
            validate_args(
                "skills:apply",
                &[
                    "--only".into(),
                    "skill:archify".into(),
                    "--action".into(),
                    "update".into(),
                    "--ref".into(),
                    reference.into(),
                ],
            )
        };
        for ok in [
            "v3.0.1",
            "release/2.x",
            "main",
            "a1b2c3d",
            "v1.0.0-rc.1+build.5",
        ] {
            assert!(update(ok).is_ok(), "{ok}");
        }
        for bad in ["--upload-pack=x", "", "a..b", "v1 ; rm", "v*"] {
            assert!(update(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn skills_adopt_commands_are_allowed_and_validated() {
        assert!(validate_command("skills:adopt-preview").is_ok());
        assert!(validate_command("skills:adopt").is_ok());
        assert!(
            validate_args("skills:adopt-preview", &["--name".into(), "archify".into()]).is_ok()
        );
        assert!(validate_args(
            "skills:adopt-preview",
            &[
                "--name".into(),
                "archify".into(),
                "--dest".into(),
                "~/skills/my-skills".into()
            ]
        )
        .is_ok());
        let digest = "a".repeat(64);
        assert!(validate_args(
            "skills:adopt",
            &[
                "--name".into(),
                "archify".into(),
                "--digest".into(),
                digest.clone(),
                "--ref".into(),
                "v3.0.1".into(),
                "--path".into(),
                "skills/archify".into(),
                "--accept-source".into()
            ],
        )
        .is_ok());
        assert!(validate_args(
            "skills:adopt",
            &["--undo".into(), "20260929T101010123-archify".into()]
        )
        .is_ok());
        assert!(validate_args("skills:adopt", &["--digest".into(), "xyz".into()]).is_err());
        assert!(validate_args("skills:adopt", &["--path".into(), "../etc".into()]).is_err());
        assert!(validate_args("skills:adopt", &["--undo".into(), "../x".into()]).is_err());
        assert!(validate_args("skills:adopt", &["--force".into()]).is_err());
        assert!(validate_args(
            "skills:adopt",
            &["--agents".into(), "claude-code,codex".into()]
        )
        .is_ok());
        assert!(validate_args("skills:adopt", &["--agents".into(), "a,,b".into()]).is_err());
        assert!(validate_args("skills:adopt", &["--agents".into(), "a;rm".into()]).is_err());
        assert!(validate_command("runs:progress").is_ok());
        assert!(validate_args("runs:progress", &["--run".into(), "gui-abc_1".into()]).is_ok());
        assert!(validate_args("runs:progress", &["--run".into(), "../x".into()]).is_err());
    }

    #[test]
    fn programs_commands_are_whitelisted_with_typed_args() {
        for command in [
            "programs:status",
            "programs:preview",
            "programs:apply",
            "programs:declare",
        ] {
            assert!(validate_command(command).is_ok(), "{command}");
        }
        assert!(validate_command("programs:exec").is_err());
        let ok = |command: &str, a: &[&str]| {
            validate_args(
                command,
                &a.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            )
            .is_ok()
        };
        assert!(ok("programs:status", &["--rescan"]));
        assert!(ok(
            "programs:apply",
            &[
                "--program",
                "npm:@wecom/cli",
                "--action",
                "upgrade",
                "--version",
                "1.3.5"
            ]
        ));
        assert!(ok(
            "programs:apply",
            &[
                "--program",
                "npm:semver",
                "--action",
                "adopt",
                "--entry-dir",
                "~/.local/bin",
                "--allow-scripts",
                "false"
            ]
        ));
        assert!(ok(
            "programs:apply",
            &[
                "--program",
                "npm:semver",
                "--action",
                "restore",
                "--backup",
                "20260930T091207893-npm_semver"
            ]
        ));
        assert!(ok(
            "programs:declare",
            &[
                "--skill",
                "xiaoyu-bug-loop",
                "--bins",
                "[\"playwright-cli\"]"
            ]
        ));
        assert!(ok(
            "programs:declare",
            &["--server", "example-code", "--follow-latest", "true"]
        ));
        assert!(!ok(
            "programs:declare",
            &["--server", "example-code", "--follow-latest", "maybe"]
        ));
        assert!(
            !ok(
                "programs:apply",
                &["--program", "npm:x", "--action", "rm -rf"]
            ),
            "动作只能是已知几种"
        );
        assert!(
            !ok(
                "programs:apply",
                &[
                    "--program",
                    "npm:x",
                    "--action",
                    "install",
                    "--version",
                    "-g"
                ]
            ),
            "版本不能像参数"
        );
        assert!(!ok(
            "programs:apply",
            &["--program", "npm:x", "--allow-scripts", "yes"]
        ));
        assert!(!ok(
            "programs:apply",
            &["--program", "npm:x;rm", "--action", "install"]
        ));
        assert!(!ok("programs:status", &["--program", "npm:x"]));
    }
}
