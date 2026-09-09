//! Rust 原生 Skills 域（native-manager.db 自管域表，无旧 Python 依赖）。
//!
//! 职责：
//! - status（只读，零写库/零网络）：集合定义 ∪ bindings 的全集视图；
//! - plan（零写入预览）：校验动作并输出 actions 与风险 summary；
//! - apply：执行单 Skill 或集合动作，通过内容快照与安全软连接落地，支持回滚与操作审计。

mod apply;
mod paths;
mod persistence;
mod plan;
mod preflight;
mod snapshot;
mod status;

use anyhow::{bail, Result};
use serde_json::{Map, Value};
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

// 供集成测试使用的最小公开发行接口
pub use snapshot::hash_skill_dir;

pub(crate) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    match req.command.as_str() {
        "skills:status" | "skills.status" => status::build_status(ctx),
        "skills:plan" | "skills.plan" => plan::build_plan(ctx, req),
        "skills:apply" | "skills.apply" => apply::apply(ctx, req),
        other => bail!("native_skills 不支持命令 '{}'", other),
    }
}
