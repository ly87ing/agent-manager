//! MCP 清单：各助手配置文件里的实际条目 + 库里的已保存项（只读、零写）。
//!
//! status、apply 预检、secrets:status、上游探测都从这里取同一份视图。
//! 旧库（服务库定义 + mcp_bindings）在读路径上即时推导成已保存项，与写路径迁移
//! 落库的结果一致（见 legacy 模块）。

use anyhow::{anyhow, Result};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use super::codecs::json_doc_from_bytes;
use super::entry::{self, View};
use super::model::digest_of;
use super::rollback::read_existing_bytes;
use super::saved::{self, SavedRow};
use super::targets::{agent_targets, resolved_path, validate_target_path, AgentTarget, Format};
use crate::native::Context;
use crate::store;

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub name: String,
    /// 原生形态的 JSON（Codex 为 TOML 表的 JSON 映射），含文件里的全部字段。
    pub raw: Value,
    pub view: std::result::Result<View, String>,
    pub base: String,
}

#[derive(Clone, Debug)]
pub struct TargetScan {
    pub target: AgentTarget,
    pub exists: bool,
    /// 文件或路径级问题：该助手的条目一概无法列出与操作。
    pub error: Option<String>,
    pub entries: Vec<FileEntry>,
    /// 读到的原始字节（写入时作为整文件 CAS 基准）。
    pub old_bytes: Option<Vec<u8>>,
}

impl TargetScan {
    pub fn entry(&self, name: &str) -> Option<&FileEntry> {
        self.entries.iter().find(|e| e.name == name)
    }
}

pub struct Inventory {
    pub scans: Vec<TargetScan>,
    pub saved: Vec<SavedRow>,
}

impl Inventory {
    pub fn scan(&self, agent: &str) -> Option<&TargetScan> {
        self.scans.iter().find(|s| s.target.agent == agent)
    }

    pub fn saved_row(&self, server: &str, agent: &str) -> Option<&SavedRow> {
        self.saved
            .iter()
            .find(|row| row.server == server && row.agent == agent)
    }
}

fn container_key(format: &Format) -> &'static str {
    match format {
        Format::OpenCodeJson => "mcp",
        Format::CodexToml => "mcp_servers",
        _ => "mcpServers",
    }
}

/// 解析整文件，返回条目容器（名称 → 原生 JSON）与文件其余部分（用于写后自检）。
pub fn parse_container(
    format: &Format,
    path: &std::path::Path,
    bytes: Option<&[u8]>,
) -> Result<(Map<String, Value>, Map<String, Value>)> {
    let Some(bytes) = bytes else {
        return Ok((Map::new(), Map::new()));
    };
    let mut root: Map<String, Value> = match format {
        Format::ClaudeJson | Format::OpenCodeJson => json_doc_from_bytes(path, bytes)?,
        Format::CodexToml => {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| anyhow!("配置文件不是有效 UTF-8: {}", path.display()))?;
            let parsed: toml::Value = toml::from_str(text).map_err(|exc| {
                anyhow!("TOML 配置解析失败: {} ({})", path.display(), exc.message())
            })?;
            match serde_json::to_value(parsed)? {
                Value::Object(map) => map,
                _ => return Err(anyhow!("TOML 根节点不是表: {}", path.display())),
            }
        }
        Format::Unsupported(fmt) => return Err(anyhow!("不支持的 MCP 配置格式: {fmt}")),
    };
    let key = container_key(format);
    let container = match root.shift_remove(key) {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(map)) => map,
        Some(_) => {
            return Err(anyhow!(
                "配置文件中的 '{}' 不是对象/表: {}",
                key,
                path.display()
            ))
        }
    };
    Ok((container, root))
}

fn scan_target(target: &AgentTarget, shared_with: Option<&str>) -> TargetScan {
    let mut scan = TargetScan {
        target: target.clone(),
        exists: false,
        error: None,
        entries: Vec::new(),
        old_bytes: None,
    };
    if let Format::Unsupported(fmt) = &target.format {
        scan.error = Some(format!("不支持的 MCP 配置格式: {fmt}"));
        return scan;
    }
    if let Some(other) = shared_with {
        scan.error = Some(format!(
            "与助手 '{other}' 共用同一个 MCP 配置文件，为保证隔离不予操作"
        ));
        return scan;
    }
    if let Err(err) = validate_target_path(&target.path) {
        scan.error = Some(format!("{err:#}"));
        return scan;
    }
    let bytes = match read_existing_bytes(&target.path) {
        Ok(bytes) => bytes,
        Err(err) => {
            scan.error = Some(format!("{err:#}"));
            return scan;
        }
    };
    scan.exists = bytes.is_some();
    match parse_container(&target.format, &target.path, bytes.as_deref()) {
        Ok((container, _)) => {
            for (name, raw) in container {
                let view = entry::parse(&target.format, &raw);
                scan.entries.push(FileEntry {
                    base: digest_of(&raw),
                    name,
                    raw,
                    view,
                });
            }
        }
        Err(err) => {
            scan.error = Some(format!("配置文件解析失败: {err:#}"));
            return scan;
        }
    }
    scan.old_bytes = bytes;
    scan
}

/// 逐个助手扫描目标文件；一个助手出错不影响其他助手。
pub fn scan_targets(policy: &Value, home_dir: &std::path::Path) -> Vec<TargetScan> {
    scan_targets_with_context(policy, home_dir, None)
}

pub fn scan_targets_checked(ctx: &Context, policy: &Value) -> Vec<TargetScan> {
    scan_targets_with_context(policy, &ctx.home_dir, Some(ctx))
}

fn scan_targets_with_context(
    policy: &Value,
    home_dir: &std::path::Path,
    ctx: Option<&Context>,
) -> Vec<TargetScan> {
    let targets = agent_targets(policy, home_dir);
    let mut by_path: BTreeMap<std::path::PathBuf, Vec<String>> = BTreeMap::new();
    for target in &targets {
        if !target.path.as_os_str().is_empty() {
            by_path
                .entry(resolved_path(&target.path))
                .or_default()
                .push(target.agent.clone());
        }
    }
    targets
        .iter()
        .map(|target| {
            if let Some(ctx) = ctx {
                if let Err(error) = crate::native_profiles::paths::validate_account_resource_path(
                    ctx,
                    policy,
                    &target.agent,
                    &target.path,
                ) {
                    return TargetScan {
                        target: target.clone(),
                        exists: false,
                        error: Some(format!("{error:#}")),
                        entries: Vec::new(),
                        old_bytes: None,
                    };
                }
            }
            let shared = if target.path.as_os_str().is_empty() {
                None
            } else {
                by_path
                    .get(&resolved_path(&target.path))
                    .and_then(|agents| agents.iter().find(|a| **a != target.agent))
                    .cloned()
            };
            scan_target(target, shared.as_deref())
        })
        .collect()
}

/// 已保存项 = 表内行 ∪ 旧库即时推导（优先级与写路径迁移一致）。
pub fn merge_saved(mut rows: Vec<SavedRow>, legacy: Vec<SavedRow>) -> Vec<SavedRow> {
    for derived in legacy {
        let existing = rows
            .iter()
            .position(|r| r.server == derived.server && r.agent == derived.agent);
        match existing {
            // 助手快照以表内为准；未分配定义以 policy 为准（来自其后的导入或保存）。
            Some(index) if derived.agent.is_empty() => rows[index] = derived,
            Some(_) => {}
            None => rows.push(derived),
        }
    }
    rows.sort_by(|a, b| (&a.server, &a.agent).cmp(&(&b.server, &b.agent)));
    rows
}

/// 只读清单：不建库、不建表、不写任何文件。
pub fn load(ctx: &Context) -> Result<Inventory> {
    let policy = store::effective_policy(ctx)?;
    let scans = scan_targets_checked(ctx, &policy);
    let rows = saved::load_readonly(ctx)?;
    let bindings = super::legacy::load_bindings_readonly(ctx)?;
    let legacy = super::legacy::derive(&policy, &bindings, &scans);
    Ok(Inventory {
        saved: merge_saved(rows, legacy),
        scans,
    })
}
