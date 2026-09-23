//! Rust 原生 Skills 域（native-manager.db 自管域表，无旧 Python 依赖）。
//!
//! 职责：
//! - status（只读，零写库/零网络）：集合定义 ∪ bindings 的全集视图，另带内置推荐来源
//!   （`recommendations`，不是元素）与各助手技能目录的实际条目（`installed`）；
//! - apply：执行单 Skill 或集合动作，通过内容快照与安全软连接落地，支持回滚与操作审计。
//!   plan 与 --dry-run 已退役：apply 一律真实执行，经 preflight 后由调用方恒取全局锁。

mod apply;
mod discover;
mod paths;
mod persistence;
mod preflight;
mod recommend;
mod snapshot;
mod status;

use anyhow::{bail, Result};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::native::{Context, Request};

pub(crate) const EXCLUDED_DIRS: &[&str] = &[".git", "node_modules", "__pycache__", "venv", ".venv"];
pub(crate) const SKILL_ACTIONS: &[&str] = &["add", "update", "enable", "disable", "remove"];
pub(crate) const COLLECTION_ACTIONS: &[&str] = &["update", "enable", "disable", "remove"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BindingEntry {
    pub(crate) version: String,
    pub(crate) enabled: bool,
    pub(crate) assigned: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct CollectionDef {
    pub(crate) name: String,
    pub(crate) url: Option<String>,
    /// 本地来源集合的源根目录（`~/...` 或绝对路径），与 `url` 互斥。
    /// 有值即直链模式：链接直接指向源目录下的 skill，不做内容快照。
    pub(crate) source_path: Option<String>,
    pub(crate) skill_paths: Option<Vec<String>>,
    pub(crate) agents: Option<Vec<String>>,
    pub(crate) disabled: Option<Vec<String>>,
    pub(crate) enabled: bool,
    pub(crate) deleted: bool,
    pub(crate) raw: Map<String, Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct AgentTargetInfo {
    #[allow(dead_code)]
    pub(crate) id: String,
    pub(crate) enabled: bool,
    pub(crate) skills_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedTarget {
    pub(crate) collection: String,
    pub(crate) skill: Option<String>,
}

// 供 native.rs 使用的 crate 内调度接口
pub(crate) use preflight::preflight_apply;

// 供 native_upstream 探测集合仓库本地 HEAD 时定位仓库缓存目录
pub(crate) use paths::repo_base_dir;

// 供 rules 域读取来源规则：集合元数据、来源根目录与推荐来源（与技能共用一份）
pub(crate) use paths::collection_source_root;
pub(crate) use recommend::recommendations_value;
pub(crate) use status::load_metadata;

// 供只读 system:discover 复用同一份元数据/binding 视图扫描未纳管条目（零写）
pub(crate) use discover::discover as discover_unmanaged;

// 供集成测试使用的最小公开发行接口
pub use snapshot::hash_skill_dir;

/// 旧库固化、从未生效的内置集合名：读路径按推荐处理（不作为集合、不参与上游检查）。
/// 零写；集合元数据与 native_upstream 的探测范围共用这一判定。
pub(crate) fn hidden_builtin_collections(
    ctx: &Context,
    policy: &Value,
) -> Result<BTreeSet<String>> {
    Ok(recommend::unadopted_builtin_collections(
        policy,
        &persistence::load_bindings_readonly(ctx)?,
    ))
}

pub(crate) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    match req.command.as_str() {
        "skills:status" | "skills.status" => status::build_status(ctx),
        "skills:apply" | "skills.apply" => apply::apply(ctx, req),
        other => bail!("native_skills 不支持命令 '{}'", other),
    }
}
