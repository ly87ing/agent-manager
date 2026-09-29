//! MCP 目标路径、格式与路径安全校验

use anyhow::{bail, Result};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

use super::model::canonicalize_agent_id;

pub use super::codecs::{render_entry_toml, toml_basic_string, toml_key, toml_remove_server_text};
pub use super::rollback::{Mutation, RollbackGuard};

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Format {
    ClaudeJson,
    CodexToml,
    OpenCodeJson,
    Unsupported(String),
}

#[derive(Clone, Debug)]
pub struct AgentTarget {
    pub agent: String,
    pub path: PathBuf,
    pub format: Format,
    pub enabled: bool,
}

impl Format {
    /// status `targets[].format` 的稳定取值。
    pub fn label(&self) -> &'static str {
        match self {
            Format::ClaudeJson => "claude-json",
            Format::CodexToml => "codex-toml",
            Format::OpenCodeJson => "opencode-json",
            Format::Unsupported(_) => "unsupported",
        }
    }

    /// 停用方式：Claude 没有全局逐条停用字段，只能移出条目并存快照；
    /// Codex 与 OpenCode 用官方 `enabled = false`。
    pub fn disable_mode(&self) -> &'static str {
        match self {
            Format::ClaudeJson => "snapshot",
            _ => "field",
        }
    }
}

pub fn expand_home(path: &str, home_dir: &Path) -> PathBuf {
    if path == "~" {
        return home_dir.to_path_buf();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home_dir.join(rest);
    }
    PathBuf::from(path)
}

pub fn format_for_agent(agent: &str, raw_format: Option<&str>) -> Format {
    if let Some(fmt) = raw_format {
        match fmt.to_ascii_lowercase().as_str() {
            "toml" | "codex-toml" => Format::CodexToml,
            "opencode" | "opencode-json" => Format::OpenCodeJson,
            "claude-json" => Format::ClaudeJson,
            "json" if agent == "opencode" => Format::OpenCodeJson,
            "json" if agent == "claude-code" => Format::ClaudeJson,
            "json" => Format::ClaudeJson,
            other => Format::Unsupported(other.to_string()),
        }
    } else {
        match agent {
            "codex" => Format::CodexToml,
            "opencode" => Format::OpenCodeJson,
            "claude-code" => Format::ClaudeJson,
            other => Format::Unsupported(format!("unknown-agent-default:{}", other)),
        }
    }
}

/// 配置了 `mcp_config` 的助手（含实例）及其目标文件。没有 `mcp_config` 的助手
/// 不涉及 MCP，不在列表里；路径为空或格式不认识的照样列出，由调用方报错。
pub fn agent_targets(policy: &Value, home_dir: &Path) -> Vec<AgentTarget> {
    let mut targets = Vec::new();
    let defs = policy
        .get("agents")
        .and_then(|agents| agents.get("definitions"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (raw_id, def) in &defs {
        let agent = canonicalize_agent_id(raw_id);
        let Some(def_map) = def.as_object() else {
            continue;
        };
        // 已移除的账号只留删除标记（目录保留）：不再是 MCP 目标，不能被列出或写入
        if def_map.get("_delete") == Some(&Value::Bool(true)) {
            continue;
        }
        let enabled = policy
            .get("agents")
            .and_then(|a| a.get(raw_id).or_else(|| a.get(&agent)))
            .and_then(|sw| sw.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let (path_str, fmt_opt) = match def_map.get("mcp_config") {
            Some(Value::String(path)) => (path.clone(), None),
            Some(Value::Object(obj)) => (
                obj.get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                obj.get("format")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            ),
            _ => continue,
        };
        let (path, format) = if path_str.is_empty() {
            (
                PathBuf::new(),
                Format::Unsupported("empty-path".to_string()),
            )
        } else {
            (
                expand_home(&path_str, home_dir),
                format_for_agent(&agent, fmt_opt.as_deref()),
            )
        };
        targets.push(AgentTarget {
            agent,
            path,
            format,
            enabled,
        });
    }
    targets.sort_by(|a, b| a.agent.cmp(&b.agent));
    targets
}

/// 路径归一：已存在的文件取 canonicalize，不存在的按词法去掉 `.`/`..`。
/// 只用于判断两个助手是否共用同一个目标文件。
pub fn resolved_path(path: &Path) -> PathBuf {
    if let Ok(canon) = std::fs::canonicalize(path) {
        return canon;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component),
        }
    }
    normalized
}

pub fn check_symlink_metadata(path: &Path) -> Result<Option<std::fs::Metadata>> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => {
            Err(anyhow::Error::from(err).context(format!("无法读取路径元数据: {}", path.display())))
        }
    }
}

pub fn validate_target_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        bail!("目标路径为空，拒绝操作");
    }
    if let Some(meta) = check_symlink_metadata(path)? {
        if meta.file_type().is_symlink() {
            bail!("目标路径是符号链接，拒绝操作以保证安全: {}", path.display());
        }
        if meta.is_dir() {
            bail!("目标路径是目录而不是普通文件，拒绝操作: {}", path.display());
        }
        if !meta.is_file() {
            bail!("目标路径不是普通文件，拒绝操作: {}", path.display());
        }
    }
    for ancestor in path.ancestors().skip(1) {
        if ancestor.as_os_str().is_empty() || ancestor == Path::new("/") {
            continue;
        }
        #[cfg(target_os = "macos")]
        if ancestor == Path::new("/var")
            || ancestor == Path::new("/tmp")
            || ancestor == Path::new("/etc")
        {
            continue;
        }
        if let Some(meta) = check_symlink_metadata(ancestor)? {
            if meta.file_type().is_symlink() {
                bail!(
                    "目标路径的祖先包含符号链接，拒绝操作以保证安全: {}",
                    ancestor.display()
                );
            }
            if !meta.is_dir() {
                bail!("目标路径的祖先不是目录，拒绝操作: {}", ancestor.display());
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct Entry {
    pub command: String,
    pub args: Vec<String>,
    pub env: Map<String, Value>,
    pub extra: Map<String, Value>,
}
