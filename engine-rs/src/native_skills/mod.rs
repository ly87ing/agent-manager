//! Rust 原生 Skills 域（native-manager.db 自管域表，无旧 Python 依赖）。
//!
//! 职责：
//! - status（只读，零写库/零网络）：集合定义 ∪ bindings 的全集视图，另带内置推荐来源
//!   （`recommendations`，不是元素）与各助手技能目录的实际条目（`installed`）；
//! - apply：执行单 Skill 或集合动作，通过内容快照与安全软连接落地，支持回滚与操作审计。
//!   plan 与 --dry-run 已退役：apply 一律真实执行，经 preflight 后由调用方恒取全局锁。

mod adopt;
mod apply;
mod discover;
mod fork;
mod git_index;
mod git_source;
mod paths;
mod persistence;
mod preflight;
mod recommend;
mod snapshot;
mod snapshot_materialized;
mod snapshot_materialized_path;
mod snapshot_storage;
mod snapshot_v2;
mod status;
pub(crate) mod track;
pub(crate) mod versioning;

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
    /// 按定义里的 `bin` 认出的助手形态；认不出（自定义助手）为 None。
    pub(crate) kind: Option<crate::native_agent_adapters::AdapterKind>,
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
pub(crate) use paths::{expand_home, is_path_within, resolve_loose};
pub(crate) use recommend::suggested_sources;
pub(crate) use status::load_collections;

// 供只读 system:discover 复用同一份元数据/binding 视图扫描未纳管条目（零写）
pub(crate) use discover::discover as discover_unmanaged;
pub(crate) use discover::skills_in_use;

// 供集成测试使用的最小公开发行接口
pub use snapshot::hash_skill_dir;

pub(crate) fn validate_collection_restore(
    conn: &rusqlite::Connection,
    before: &Value,
    after: &Value,
) -> Result<()> {
    let before = before.pointer("/skills/repositories");
    let after = after.pointer("/skills/repositories");
    for ((collection, _, _), binding) in persistence::load_bindings_from_conn(conn)? {
        if binding.assigned
            && before.and_then(|v| v.get(&collection)) != after.and_then(|v| v.get(&collection))
        {
            bail!("来源 {collection} 仍有技能分配，不能通过恢复移除或替换；请先移除该来源的分配，再重新预览备份");
        }
    }
    Ok(())
}

pub(crate) fn validate_collection_locations(
    conn: &rusqlite::Connection,
    before: &Value,
    after: &Value,
) -> Result<()> {
    for ((collection, _, _), binding) in persistence::load_bindings_from_conn(conn)? {
        let old = before
            .pointer("/skills/repositories")
            .and_then(|v| v.get(&collection));
        let new = after
            .pointer("/skills/repositories")
            .and_then(|v| v.get(&collection));
        if binding.assigned
            && (old.is_some() != new.is_some()
                || ["source_path", "url", "_delete"]
                    .iter()
                    .any(|key| old.and_then(|v| v.get(key)) != new.and_then(|v| v.get(key))))
        {
            bail!("来源 {collection} 仍有技能分配，不能移除或更换来源目录；请先移除该来源的分配");
        }
    }
    Ok(())
}

fn orphaned_collection(name: &str) -> CollectionDef {
    CollectionDef {
        name: name.into(),
        url: None,
        source_path: None,
        skill_paths: None,
        agents: None,
        disabled: None,
        enabled: false,
        deleted: true,
        raw: Map::new(),
    }
}

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
        "skills:status" | "skills.status" => {
            if req
                .args
                .iter()
                .any(|arg| !matches!(arg.as_str(), "--collections-only" | "--offline"))
            {
                bail!("skills:status 仅接受 --collections-only 或 --offline");
            }
            status::build_status_view(ctx, req.flag("--collections-only"))
        }
        "skills:apply" | "skills.apply" => apply::apply(ctx, req),
        "skills:fork" | "skills.fork" => fork::fork(ctx, req),
        "skills:versions" | "skills.versions" => versions_command(ctx, req),
        "skills:preview" | "skills.preview" => preview_command(ctx, req),
        "skills:adopt-preview" | "skills.adopt-preview" => adopt::preview(ctx, req),
        "skills:adopt" | "skills.adopt" => adopt::apply(ctx, req),
        other => bail!("native_skills 不支持命令 '{}'", other),
    }
}

/// `skills:versions --only skill:<来源>`：列出 git 来源可选的版本（标签、分支）、按跟踪方式
/// 解析出的最新版本与镜像当前所在版本。只读网络：不写库、不动镜像，也不取 apply 锁。
fn versions_command(ctx: &Context, req: &Request) -> Result<Value> {
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--only" => {
                iter.next();
            }
            other => bail!("skills:versions 只接受 --only，收到 {other}"),
        }
    }
    let only = req
        .value("--only")
        .ok_or_else(|| anyhow::anyhow!("skills:versions 需要 --only skill:<来源>"))?;
    let target = paths::parse_only(only)?;
    if target.skill.is_some() {
        bail!("skills:versions 只接受来源（skill:<来源>），不接受单个技能");
    }
    let meta = status::load_metadata(ctx)?;
    let coll = meta
        .collections
        .get(&target.collection)
        .filter(|coll| !coll.deleted)
        .ok_or_else(|| anyhow::anyhow!("没有这个来源：{}", target.collection))?;
    versioning::versions(ctx, coll, &paths::collection_source_root(ctx, coll))
}

/// `skills:preview --only skill:<来源> [--ref <版本>]`：换版本前列出各技能的文件变化与会跟着换的账号。
/// 只把目标提交取进本工具的镜像缓存；不检出、不动账号目录、不写库、不取 apply 锁。
fn preview_command(ctx: &Context, req: &Request) -> Result<Value> {
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--only" | "--ref" => {
                iter.next();
            }
            other => bail!("skills:preview 只接受 --only 与 --ref，收到 {other}"),
        }
    }
    if let Some(reference) = req.value("--ref") {
        if !track::valid_ref_name(reference) || reference.contains('*') {
            bail!("--ref 不合法：{reference}");
        }
    }
    let only = req
        .value("--only")
        .ok_or_else(|| anyhow::anyhow!("skills:preview 需要 --only skill:<来源>"))?;
    let target = paths::parse_only(only)?;
    if target.skill.is_some() {
        bail!("skills:preview 只接受来源（skill:<来源>），不接受单个技能");
    }
    let meta = status::load_metadata(ctx)?;
    let coll = meta
        .collections
        .get(&target.collection)
        .filter(|coll| !coll.deleted)
        .ok_or_else(|| anyhow::anyhow!("没有这个来源：{}", target.collection))?;
    let mut in_use: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for ((c, skill, agent), binding) in persistence::load_bindings_readonly(ctx)? {
        if c == coll.name && binding.assigned && binding.enabled {
            in_use.entry(skill).or_default().push(agent);
        }
    }
    versioning::preview_update(
        ctx,
        coll,
        &paths::collection_source_root(ctx, coll),
        req.value("--ref"),
        &in_use,
    )
}
