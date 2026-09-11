//! MCP 目标路径、格式与原子文件写入/回读校验

use anyhow::{bail, Result};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::model::{canonicalize_agent_id, compute_applied_digest};

pub use super::codecs::{
    extract_entry, json_remove_entry, json_set_entry, read_json_doc, render_entry_toml,
    toml_basic_string, toml_key, toml_remove_server_text,
};
pub use super::rollback::{atomic_write_checked, Mutation, RollbackGuard};

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

pub fn agent_targets(policy: &Value, home_dir: &Path) -> Result<(Vec<AgentTarget>, Vec<String>)> {
    let mut targets = Vec::new();
    let mut disabled = Vec::new();
    let agents = policy.get("agents").cloned().unwrap_or(Value::Null);
    let defs = agents
        .get("definitions")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (raw_id, def) in &defs {
        let agent = canonicalize_agent_id(raw_id);
        let def_map = match def.as_object() {
            Some(map) => map,
            None => continue,
        };
        let enabled = policy
            .get("agents")
            .and_then(|a| a.get(raw_id).or_else(|| a.get(&agent)))
            .and_then(|sw| sw.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let mcp_config = match def_map.get("mcp_config") {
            Some(Value::String(path)) => Some((path.clone(), None)),
            Some(Value::Object(obj)) => {
                let p = obj
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let fmt = obj
                    .get("format")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                Some((p, fmt))
            }
            _ => None,
        };
        let Some((path_str, fmt_opt)) = mcp_config else {
            targets.push(AgentTarget {
                agent: agent.clone(),
                path: PathBuf::new(),
                format: Format::Unsupported("missing-mcp-config".to_string()),
                enabled,
            });
            if !enabled {
                disabled.push(agent);
            }
            continue;
        };
        if path_str.is_empty() {
            targets.push(AgentTarget {
                agent: agent.clone(),
                path: PathBuf::new(),
                format: Format::Unsupported("empty-path".to_string()),
                enabled,
            });
            if !enabled {
                disabled.push(agent);
            }
            continue;
        }
        let format = format_for_agent(&agent, fmt_opt.as_deref());
        let target = AgentTarget {
            agent: agent.clone(),
            path: expand_home(&path_str, home_dir),
            format,
            enabled,
        };
        if enabled {
            targets.push(target);
        } else {
            disabled.push(agent);
            targets.push(target);
        }
    }
    targets.sort_by(|a, b| a.agent.cmp(&b.agent));
    Ok((targets, disabled))
}

pub fn check_shared_targets(targets: &[AgentTarget]) -> Result<()> {
    let mut seen: BTreeMap<PathBuf, String> = BTreeMap::new();
    for target in targets {
        if target.path.as_os_str().is_empty() {
            continue;
        }
        let resolved = if let Ok(canon) = std::fs::canonicalize(&target.path) {
            canon
        } else {
            let mut normalized = PathBuf::new();
            for component in target.path.components() {
                match component {
                    std::path::Component::CurDir => {}
                    std::path::Component::ParentDir => {
                        normalized.pop();
                    }
                    _ => normalized.push(component),
                }
            }
            normalized
        };
        if let Some(other) = seen.get(&resolved) {
            bail!(
                "Agent '{}' 与 '{}' 共享 MCP 目标 {}，拒绝操作以保证隔离",
                target.agent,
                other,
                target.path.display()
            );
        }
        seen.insert(resolved, target.agent.clone());
    }
    Ok(())
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

impl Entry {
    pub fn digest(&self) -> String {
        compute_applied_digest(
            &Value::String(self.command.clone()),
            &Value::Array(self.args.iter().cloned().map(Value::String).collect()),
            &Value::Object(self.env.clone()),
            &Value::Object(self.extra.clone()),
        )
    }
}
