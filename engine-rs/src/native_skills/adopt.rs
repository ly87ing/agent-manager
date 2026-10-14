//! 纳入管理：把别的方式装进账号目录的技能（「其他来源」）交给本工具管理，只留一份源。
//!
//! - `skills:adopt-preview --name <技能>`：只读（不写库、不动账号目录与本工具的数据目录）。识别来源，
//!   在系统临时目录取来源的版本与现有内容逐文件比对；本地是旧版本时找对应的版本号标签，按那个版本
//!   纳入（之后显示「可更新」）。给出要备份、删除、链接的路径与账号，以及一个指纹 `digest`。
//! - `skills:adopt --name <技能> --digest <指纹> [--ref <版本>] [--keep-local] [--accept-source]`：
//!   重算本地内容与远端目标，指纹对不上就拒绝（预览后内容变了）。先把原目录 / 链接移进
//!   `adopt-backups/<id>/` 并写备份清单，再登记来源、下载、链接到同类的全部账号；任何一步失败都按
//!   清单原样恢复。`--keep-local` 默认复制到 `~/skills/my-skills/<技能>`，也可用 `--dest` 指定集合目录。
//! - 已被本工具管理、只是在 Codex 共用目录（`~/.agents/skills`）里还有一份的，纳入只做清理。
//! - `skills:adopt --undo <id>`：按清单撤回——解除这次的链接与登记，把原目录放回原处。
//!
//! 不改别的工具（`npx skills`）的锁文件；Claude 自己同步的 `synced/` 等不可纳入的项一律拒绝。

use anyhow::{anyhow, bail, Context as _, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::paths::{expand_home, personal_source_path, resolve_loose};
use super::persistence::load_bindings_readonly;
use super::snapshot::{copy_dir_recursive, hash_skill_dir};
use super::status::load_metadata;
use super::track::{self, Track};
use super::versioning;
use super::{CollectionDef, EXCLUDED_DIRS};
use crate::native::{Context, Request};
use crate::progress::Progress;
use crate::store;

const BACKUP_DIR: &str = "adopt-backups";
const MANIFEST: &str = "manifest.json";
/// 找「本地是哪个旧版本」时最多比较的最近正式版本数。
const RECENT_VERSIONS: usize = 20;

/// 一处要处理的位置：账号技能目录里的条目，或 Codex 共用目录里的条目。
#[derive(Debug, Clone)]
struct Location {
    agent: Option<String>,
    shared: Option<String>,
    path: PathBuf,
    origin: String,
    /// 内容所在目录：真实目录是它自己，链接是它指向的目录。
    content: Option<PathBuf>,
    hash: Option<String>,
}

impl Location {
    fn to_json(&self, home: &Path) -> Value {
        json!({
            "agent": self.agent,
            "shared": self.shared,
            "path": tilde(&self.path, home),
            "origin": self.origin,
        })
    }
}

/// 本地一侧的计划（便宜、可重算）：处理方式、位置、来源、已管理它的来源。
struct LocalPlan {
    name: String,
    /// `dedupe`：已被管理，只清理共用目录里的副本；`adopt`：纳入。
    kind: &'static str,
    locations: Vec<Location>,
    source: Value,
    managed_by: Option<String>,
}

fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rel) => format!("~/{}", crate::platform::slash(&rel)),
        Err(_) => path.to_string_lossy().to_string(),
    }
}

fn valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && !name.starts_with('.')
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn local_plan(ctx: &Context, name: &str) -> Result<LocalPlan> {
    if !valid_skill_name(name) {
        bail!("技能名不合法：{name}");
    }
    let meta = load_metadata(ctx)?;
    let bindings = load_bindings_readonly(ctx)?;
    let view = super::discover::unmanaged(ctx, &meta, &bindings);
    let home = &ctx.home_dir;
    let expand = |raw: &str| expand_home(raw, home);
    let managed_by = bindings
        .iter()
        .find(|((_, skill, _), b)| skill == name && b.assigned)
        .map(|((coll, _, _), _)| coll.clone());
    let entry = view["skills"]
        .as_array()
        .and_then(|skills| skills.iter().find(|s| s["name"] == name))
        .cloned();
    let mut locations: Vec<Location> = Vec::new();
    fn push(
        locations: &mut Vec<Location>,
        agent: Option<String>,
        shared: Option<String>,
        path: PathBuf,
        origin: &str,
        target: Option<PathBuf>,
    ) -> Result<()> {
        let content = if origin == "native-dir" {
            Some(path.clone())
        } else {
            target
        };
        let hash = match &content {
            Some(dir) if dir.join("SKILL.md").is_file() => Some(hash_skill_dir(dir)?),
            _ => None,
        };
        locations.push(Location {
            agent,
            shared,
            path,
            origin: origin.to_string(),
            content,
            hash,
        });
        Ok(())
    }
    if let Some(managed) = &managed_by {
        if entry.is_some() {
            bail!("账号目录里还有一个本工具没管理的 {name}，与来源 {managed} 里的同名技能冲突；请先手动处理");
        }
        for dup in view["duplicates"].as_array().into_iter().flatten() {
            if dup["name"] != name {
                continue;
            }
            let shared_path = expand(dup["paths"][1].as_str().unwrap_or_default());
            if locations.iter().any(|l| l.path == shared_path) {
                continue;
            }
            let meta = std::fs::symlink_metadata(&shared_path)
                .with_context(|| format!("读不到 {}", shared_path.display()))?;
            let target = meta
                .file_type()
                .is_symlink()
                .then(|| crate::platform::read_link(&shared_path).ok())
                .flatten()
                .map(|t| resolve_loose(&shared_path.parent().unwrap_or(home).join(t)));
            let origin = if meta.file_type().is_symlink() {
                "external-link"
            } else {
                "native-dir"
            };
            push(
                &mut locations,
                None,
                Some("codex-shared".into()),
                shared_path,
                origin,
                target,
            )?;
        }
        if locations.is_empty() {
            bail!("{name} 已由来源 {managed} 管理，没有多余的副本要清理");
        }
        return Ok(LocalPlan {
            name: name.to_string(),
            kind: "dedupe",
            locations,
            source: Value::Null,
            managed_by,
        });
    }
    let entry = entry.ok_or_else(|| anyhow!("没有找到本工具没管理的技能 {name}"))?;
    if entry["adoptable"] != true {
        bail!(
            "{name} 不能纳入管理（{}）",
            entry["blocked_reason"].as_str().unwrap_or("未知原因")
        );
    }
    for loc in entry["locations"].as_array().into_iter().flatten() {
        push(
            &mut locations,
            loc["agent"].as_str().map(str::to_string),
            loc["shared"].as_str().map(str::to_string),
            expand(loc["path"].as_str().unwrap_or_default()),
            loc["origin"].as_str().unwrap_or("native-dir"),
            loc["target"].as_str().map(|t| expand(t)),
        )?;
    }
    Ok(LocalPlan {
        name: name.to_string(),
        kind: "adopt",
        locations,
        source: entry["source"].clone(),
        managed_by: None,
    })
}

/// 比较两个技能目录，列出增删改的文件（依赖目录与隐藏的 .git 不算）。
fn diff_dirs(ours: &Path, theirs: &Path) -> Vec<Value> {
    fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in read.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                let Ok(meta) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                if meta.is_dir() {
                    if !EXCLUDED_DIRS.contains(&name.as_str()) {
                        stack.push(path);
                    }
                } else if let Ok(rel) = path.strip_prefix(root) {
                    let bytes = if meta.file_type().is_symlink() {
                        crate::platform::read_link(&path)
                            .map(|t| t.to_string_lossy().as_bytes().to_vec())
                            .unwrap_or_default()
                    } else {
                        std::fs::read(&path).unwrap_or_default()
                    };
                    out.insert(rel.to_string_lossy().replace('\\', "/"), bytes);
                }
            }
        }
        out
    }
    let a = files(ours);
    let b = files(theirs);
    let mut out = Vec::new();
    // 本地内容为改前、来源内容为改后（纳入时按来源的版本）；前 DIFF_FILE_COUNT 个给出原文
    let mut push = |status: &str, file: &str, local: Option<&Vec<u8>>, source: Option<&Vec<u8>>| {
        let mut entry = json!({"status": status, "file": file});
        if out.len() < versioning::DIFF_FILE_COUNT {
            versioning::attach_contents(&mut entry, local.cloned(), source.cloned());
        } else {
            entry["contents"] = json!(false);
        }
        out.push(entry);
    };
    for (file, bytes) in &a {
        match b.get(file) {
            None => push("only-local", file, Some(bytes), None),
            Some(other) if other != bytes => push("modified", file, Some(bytes), Some(other)),
            _ => {}
        }
    }
    for (file, bytes) in b.iter().filter(|(f, _)| !a.contains_key(*f)) {
        push("only-source", file, None, Some(bytes));
    }
    out
}

/// 仓库地址归一：去掉协议、`.git`、结尾斜杠，小写主机名，用来判断「同一个仓库」。
fn same_repo(a: &str, b: &str) -> bool {
    let norm = |url: &str| {
        let mut s = url
            .trim()
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .to_string();
        for prefix in ["https://", "http://", "ssh://", "git://", "file://"] {
            if let Some(rest) = s.strip_prefix(prefix) {
                s = rest.to_string();
            }
        }
        if let Some(rest) = s.strip_prefix("git@") {
            s = rest.replacen(':', "/", 1);
        }
        s.to_lowercase()
    };
    norm(a) == norm(b)
}

/// 要纳入的来源：已登记的同一仓库（沿用它的名字与跟踪方式）或新建的名字。
fn target_collection(
    meta_collections: &BTreeMap<String, CollectionDef>,
    repository: &str,
    wanted: Option<&str>,
) -> Result<(String, Option<CollectionDef>)> {
    if let Some(existing) = meta_collections
        .values()
        .find(|c| !c.deleted && c.url.as_deref().is_some_and(|u| same_repo(u, repository)))
    {
        return Ok((existing.name.clone(), Some(existing.clone())));
    }
    let base = match wanted {
        Some(name) => name.to_string(),
        None => {
            let last = repository
                .trim_end_matches('/')
                .trim_end_matches(".git")
                .rsplit(['/', ':'])
                .next()
                .unwrap_or("skills")
                .to_lowercase();
            let slug: String = last
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect();
            let slug = slug.trim_matches('-').to_string();
            if slug.is_empty() {
                "skills".to_string()
            } else {
                slug
            }
        }
    };
    if !base
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        || base.starts_with('-')
    {
        bail!("来源名只能用小写字母、数字和连字符：{base}");
    }
    let mut name = base.clone();
    let mut n = 2;
    while meta_collections.get(&name).is_some_and(|c| !c.deleted) {
        if wanted.is_some() {
            bail!("来源名 {name} 已被别的仓库占用");
        }
        name = format!("{base}-{n}");
        n += 1;
    }
    Ok((name, None))
}

/// 在系统临时目录只取来源里这个技能的目录，比对后删掉。返回比对结果与建议纳入的版本。
struct SourceCheck {
    path: String,
    adopt_at: track::Target,
    latest: track::Target,
    /// 每个位置与所选版本是否一致，以及不一致的文件。
    per_location: Vec<(bool, Vec<Value>)>,
}

fn temp_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "agent-manager-adopt-{}-{nanos:x}",
        std::process::id()
    ))
}

fn check_source(
    ctx: &Context,
    plan: &LocalPlan,
    repository: &str,
    track: &Track,
    fixed: Option<&str>,
    progress: &mut Progress,
) -> Result<SourceCheck> {
    progress.start("resolve", Some(repository.to_string()));
    let refs = crate::native_upstream::git_remote_refs(ctx, repository)
        .map_err(|_| anyhow!("读不到 {repository} 的分支与标签（网络、地址或权限问题）"))?;
    let latest = track::resolve(track, &refs)?;
    // 并进已登记的来源：按它现在所在的版本比，不因为纳入一个技能就挪动整个来源
    let fixed = fixed.map(|commit| track::Target {
        kind: if refs.tag_at(commit).is_some() {
            "tag"
        } else {
            "commit"
        },
        name: refs.tag_at(commit).unwrap_or_else(|| commit.to_string()),
        commit: commit.to_string(),
    });
    let tmp = temp_dir();
    progress.start("fetch", Some("只取这一个目录；仓库很大时要久一些".into()));
    let result = (|| -> Result<SourceCheck> {
        let status = crate::platform::git_command()
            .args([
                "clone",
                "--quiet",
                "--filter=tree:0",
                "--no-checkout",
                "--",
                repository,
            ])
            .arg(&tmp)
            .env("GIT_TERMINAL_PROMPT", "0")
            .status()
            .context("git clone 启动失败")?;
        if !status.success() {
            bail!("取不到来源仓库 {repository}");
        }
        let git = |args: &[&str]| versioning::git_in(&tmp, args, &plan.name);
        // 技能在仓库里的位置：来源记录里有就用，否则按名字在最新版本里找
        let path = match plan.source["path"].as_str().filter(|p| !p.is_empty()) {
            Some(path) => path.trim_matches('/').to_string(),
            None => versioning::available_at(&tmp, &latest.commit, &plan.name)?
                .into_iter()
                .find(|item| item["name"] == plan.name.as_str())
                .and_then(|item| item["path"].as_str().map(str::to_string))
                .ok_or_else(|| anyhow!("来源仓库里找不到技能 {}", plan.name))?,
        };
        git(&["sparse-checkout", "set", "--no-cone", &format!("/{path}")])?;
        let compare = |target: &track::Target| -> Result<Vec<(bool, Vec<Value>)>> {
            git(&[
                "-c",
                "advice.detachedHead=false",
                "checkout",
                "--quiet",
                "--force",
                "--detach",
                &target.commit,
            ])?;
            let dir = tmp.join(&path);
            if !dir.join("SKILL.md").is_file() {
                return Ok(plan.locations.iter().map(|_| (false, Vec::new())).collect());
            }
            let theirs = hash_skill_dir(&dir)?;
            Ok(plan
                .locations
                .iter()
                .map(|loc| match (&loc.hash, &loc.content) {
                    (Some(hash), _) if *hash == theirs => (true, Vec::new()),
                    (_, Some(ours)) => (false, diff_dirs(ours, &dir)),
                    _ => (false, Vec::new()),
                })
                .collect())
        };
        if let Some(fixed) = fixed {
            let at_fixed = compare(&fixed)?;
            return Ok(SourceCheck {
                path,
                adopt_at: fixed,
                latest,
                per_location: at_fixed,
            });
        }
        progress.start("compare", Some(format!("对照 {}", latest.label())));
        let at_latest = compare(&latest)?;
        if at_latest.iter().all(|(same, _)| *same) {
            return Ok(SourceCheck {
                path,
                adopt_at: latest.clone(),
                latest,
                per_location: at_latest,
            });
        }
        // 本地可能是旧版本：先试记录的版本号，再从新到旧试最近的正式版本标签，找内容一致的那个
        let mut candidates: Vec<String> = Vec::new();
        if let Some(version) = plan.source["version"].as_str() {
            let bare = version.trim_start_matches('v');
            candidates.extend([format!("v{bare}"), bare.to_string()]);
        }
        let mut versions: Vec<(track::Version, &String)> = refs
            .tags
            .keys()
            .filter_map(|name| track::Version::parse(name).map(|v| (v, name)))
            .filter(|(v, _)| !v.is_prerelease())
            .collect();
        versions.sort_by(|a, b| b.0.cmp(&a.0));
        candidates.extend(
            versions
                .into_iter()
                .take(RECENT_VERSIONS)
                .map(|(_, name)| name.clone()),
        );
        // 先比目录树指纹（只要目录结构，不下载文件内容），对上了再取那一个版本逐文件确认
        let local_trees: Vec<Option<String>> = plan
            .locations
            .iter()
            .map(|loc| {
                loc.content
                    .as_deref()
                    .and_then(|dir| local_tree_id(&tmp, dir))
            })
            .collect();
        let mut tried: Vec<String> = Vec::new();
        let total = candidates.len();
        for (index, tag) in candidates.into_iter().enumerate() {
            progress.detail(
                "compare",
                format!("本地可能是旧版本，逐个对照：{}/{total}（{tag}）", index + 1),
            );
            let Some(commit) = refs.tags.get(&tag) else {
                continue;
            };
            if *commit == latest.commit || tried.contains(commit) {
                continue;
            }
            tried.push(commit.clone());
            let theirs = git(&[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{commit}:{path}"),
            ])
            .ok()
            .map(|out| out.trim().to_string());
            if theirs.is_none()
                || local_trees
                    .iter()
                    .any(|ours| ours.as_ref() != theirs.as_ref())
            {
                continue;
            }
            let target = track::Target {
                kind: "tag",
                name: tag.clone(),
                commit: commit.clone(),
            };
            let at_tag = compare(&target)?;
            if at_tag.iter().all(|(same, _)| *same) {
                return Ok(SourceCheck {
                    path,
                    adopt_at: target,
                    latest,
                    per_location: at_tag,
                });
            }
        }
        // 都对不上：内容以最新版本为准比较，差异交给用户决定
        compare(&latest)?;
        Ok(SourceCheck {
            path,
            adopt_at: latest.clone(),
            latest,
            per_location: at_latest,
        })
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// 本地目录按 git 的方式算出的目录树 ID（对象写进临时仓库，不访问网络）；算不出时为 None。
fn local_tree_id(tmp_repo: &Path, dir: &Path) -> Option<String> {
    let index = tmp_repo.join(".git").join(format!("am-index-{:x}", {
        let mut h = Sha256::new();
        h.update(dir.to_string_lossy().as_bytes());
        h.finalize()[0]
    }));
    let git = |args: &[&str]| {
        crate::platform::git_command()
            // 临时仓库开着稀疏检出，会拒绝加入范围外的路径；这里只是算树 ID，关掉它
            .args(["-c", "core.sparseCheckout=false", "--git-dir"])
            .arg(tmp_repo.join(".git"))
            .arg("--work-tree")
            .arg(dir)
            .current_dir(dir)
            .args(args)
            .env("GIT_INDEX_FILE", &index)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .ok()
            .filter(|out| out.status.success())
    };
    git(&["add", "--all", "--force", "."])?;
    let out = git(&["write-tree"])?;
    let _ = std::fs::remove_file(&index);
    let tree = String::from_utf8_lossy(&out.stdout).trim().to_string();
    track::is_sha(&tree).then_some(tree)
}

/// 来源一侧参与指纹的内容：仓库、路径、指定的版本、目标提交，以及是否覆盖本地改动。
fn fingerprint(
    repository: &str,
    path: &str,
    reference: Option<&str>,
    commit: &str,
    overwrite: bool,
) -> Value {
    json!({"repository": repository, "path": path, "ref": reference, "commit": commit, "overwrite": overwrite})
}

/// 指纹：本地内容、处理方式与目标版本；执行时重算，对不上就拒绝。
fn digest(plan: &LocalPlan, mode: &str, target: &Value) -> String {
    let body = json!({
        "name": plan.name,
        "kind": plan.kind,
        "mode": mode,
        "locations": plan.locations.iter().map(|l| json!({
            "path": l.path.to_string_lossy(),
            "origin": l.origin,
            "hash": l.hash,
            "content": l.content.as_ref().map(|c| c.to_string_lossy().to_string()),
        })).collect::<Vec<_>>(),
        "target": target,
    });
    let mut h = Sha256::new();
    h.update(body.to_string().as_bytes());
    format!("{:x}", h.finalize())
}

/// 纳入后要链接到的账号：已启用、认得形态（Claude / Codex）的全部账号，受来源的 agents 限制。
fn link_agents(ctx: &Context, restrict: Option<&Vec<String>>) -> Result<Vec<String>> {
    let meta = load_metadata(ctx)?;
    Ok(meta
        .agents
        .iter()
        .filter(|(id, info)| {
            info.enabled && info.kind.is_some() && restrict.is_none_or(|r| r.contains(id))
        })
        .map(|(id, _)| id.clone())
        .collect())
}

fn local_destination(ctx: &Context, req: &Request) -> Result<(String, Value)> {
    let meta = load_metadata(ctx)?;
    let collection = super::fork::DEFAULT_COLLECTION;
    let raw = personal_source_path(ctx, &meta, collection, req.value("--dest"))?;
    let fingerprint = json!({
        "collection": collection,
        "path": crate::native_profiles::paths::resolved(&expand_home(&raw, &ctx.home_dir))?,
        "definition": meta.collections.get(collection).filter(|c| !c.deleted).map(|c| &c.raw),
    });
    Ok((raw, fingerprint))
}

pub(crate) fn preview(ctx: &Context, req: &Request) -> Result<Value> {
    strict_args(
        req,
        &["--name", "--collection", "--dest"],
        &["--local-only"],
    )?;
    let name = req
        .value("--name")
        .ok_or_else(|| anyhow!("需要 --name <技能>"))?;
    let plan = local_plan(ctx, name)?;
    let home = &ctx.home_dir;
    // 原来就有它的账号：它所在的账号目录，以及共用目录对应的全部 Codex 账号
    let current_accounts: Vec<String> = {
        let meta = load_metadata(ctx)?;
        meta.agents
            .iter()
            .filter(|(id, info)| {
                info.enabled
                    && plan.locations.iter().any(|loc| {
                        loc.agent.as_deref() == Some(id.as_str())
                            || (loc.shared.is_some()
                                && info.kind
                                    == Some(crate::native_agent_adapters::AdapterKind::Codex))
                    })
            })
            .map(|(id, _)| id.clone())
            .collect()
    };
    let mut out = json!({
        "name": plan.name,
        "kind": plan.kind,
        "current_accounts": current_accounts,
        "locations": plan.locations.iter().map(|l| l.to_json(home)).collect::<Vec<_>>(),
        "source": plan.source,
        "removes": plan.locations.iter().map(|l| tilde(&l.path, home)).collect::<Vec<_>>(),
    });
    if plan.kind == "dedupe" {
        out["collection"] = json!({"name": plan.managed_by, "exists": true});
        out["digest"] = json!(digest(&plan, "dedupe", &Value::Null));
        return Ok(out);
    }
    match local_destination(ctx, req) {
        Ok((raw, fingerprint)) => {
            let dest = expand_home(&raw, home).join(&plan.name);
            out["local"] = json!({
            "dest": tilde(&dest, home),
            "dest_exists": std::fs::symlink_metadata(&dest).is_ok(),
            "collection": super::fork::DEFAULT_COLLECTION,
            "root": raw,
            "digest": digest(&plan, "local", &fingerprint),
            "accounts": link_agents(ctx, None)?,
            // 保留本地内容时以哪一份为准：第一处有 SKILL.md 的
            "from": plan.locations.iter().find(|l| l.hash.is_some()).map(|l| tilde(&l.path, home)),
            "variants": plan.locations.iter().filter_map(|l| l.hash.clone()).collect::<std::collections::BTreeSet<_>>().len(),
                });
        }
        Err(error) => out["local_error"] = json!(error.to_string()),
    }
    if req.flag("--local-only") {
        return Ok(out);
    }
    let Some(repository) = plan.source["repository"].as_str().map(str::to_string) else {
        out["source_check"] = json!({"error": "认不出来源仓库，只能作为本机来源纳入"});
        return Ok(out);
    };
    let meta = load_metadata(ctx)?;
    let (coll_name, existing) =
        target_collection(&meta.collections, &repository, req.value("--collection"))?;
    let track = match &existing {
        Some(coll) => versioning::track_of(coll)?,
        None => Track::Auto,
    };
    let fixed = existing
        .as_ref()
        .and_then(|coll| versioning::head_commit(&super::paths::collection_source_root(ctx, coll)));
    let mut progress = Progress::new(
        &req.run_id,
        &[
            ("resolve", "读取来源的分支与标签"),
            ("fetch", "取来源里的这个技能"),
            ("compare", "与现有内容比对"),
        ],
    );
    match check_source(
        ctx,
        &plan,
        &repository,
        &track,
        fixed.as_deref(),
        &mut progress,
    ) {
        Ok(check) => {
            let all_same = check.per_location.iter().all(|(same, _)| *same);
            // 已登记的来源固定在它现在的提交；新来源只有「本地是旧版本」时才指定版本
            let reference = if fixed.is_some() {
                Some(check.adopt_at.commit.clone())
            } else {
                (check.adopt_at.commit != check.latest.commit).then(|| check.adopt_at.name.clone())
            };
            let target = fingerprint(
                &repository,
                &check.path,
                reference.as_deref(),
                &check.adopt_at.commit,
                !all_same,
            );
            out["collection"] = json!({
                "name": coll_name,
                "exists": existing.is_some(),
                "url": repository,
                "path": check.path,
                // 纳入后账号里的链接名：来源里的目录名（本地可能换过名字）
                "linked_as": check.path.rsplit('/').next().unwrap_or(&plan.name),
                "track": existing.as_ref().and_then(|c| c.raw.get("track").cloned()).unwrap_or(Value::Null),
            });
            out["source_check"] = json!({
                "same": all_same,
                "path": check.path,
                "adopt_at": check.adopt_at.to_json(),
                "latest": check.latest.to_json(),
                "ref": reference,
                "locations": check.per_location.iter().map(|(same, files)| json!({"same": same, "files": files})).collect::<Vec<_>>(),
                "accounts": link_agents(ctx, existing.as_ref().and_then(|c| c.agents.as_ref()))?,
                "digest": digest(&plan, "source", &target),
            });
        }
        Err(err) => {
            out["source_check"] = json!({"error": format!("{err:#}")});
        }
    }
    Ok(out)
}

fn strict_args(req: &Request, values: &[&str], flags: &[&str]) -> Result<()> {
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        if values.contains(&arg.as_str()) {
            if iter.next().is_none() {
                bail!("{arg} 缺少取值");
            }
        } else if !flags.contains(&arg.as_str()) {
            bail!("{} 不接受参数 {arg}", req.command);
        }
    }
    Ok(())
}

/// 锁前预检：参数形状（不读网络、不写）。
pub(crate) fn preflight(ctx: &Context, req: &Request) -> Result<()> {
    strict_args(
        req,
        &[
            "--name",
            "--digest",
            "--ref",
            "--collection",
            "--path",
            "--agents",
            "--undo",
            "--dest",
        ],
        &["--keep-local", "--accept-source"],
    )?;
    if let Some(id) = req.value("--undo") {
        if req.args.len() != 2 {
            bail!("--undo 不能和别的参数一起用");
        }
        manifest_path(ctx, id)?;
        return Ok(());
    }
    let name = req
        .value("--name")
        .ok_or_else(|| anyhow!("需要 --name <技能>"))?;
    if !valid_skill_name(name) {
        bail!("技能名不合法：{name}");
    }
    req.value("--digest")
        .ok_or_else(|| anyhow!("需要 --digest（先预览）"))?;
    if req.value("--dest").is_some() && !req.flag("--keep-local") {
        bail!("--dest 只用于保留本地内容（--keep-local）");
    }
    if let Some(reference) = req.value("--ref") {
        if !track::valid_ref_name(reference) || reference.contains('*') {
            bail!("--ref 不合法：{reference}");
        }
    }
    if let Some(agents) = req.value("--agents") {
        let list: Vec<&str> = agents.split(',').collect();
        if list.iter().any(|a| {
            a.is_empty()
                || !a
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        }) {
            bail!("--agents 要写成逗号分隔的账号 id，至少一个");
        }
    }
    if let Some(path) = req.value("--path") {
        if path.starts_with('-')
            || path.starts_with('/')
            || std::path::Path::new(path).is_absolute()
            || path.contains('\\')
            || path.split('/').any(|p| p == ".." || p.is_empty())
        {
            bail!("--path 不合法：{path}");
        }
    }
    if req.flag("--keep-local") && (req.flag("--accept-source") || req.value("--ref").is_some()) {
        bail!("--keep-local 不能与 --accept-source / --ref 同用");
    }
    Ok(())
}

fn manifest_path(ctx: &Context, id: &str) -> Result<PathBuf> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("备份编号不合法：{id}");
    }
    let path = ctx.data_dir.join(BACKUP_DIR).join(id).join(MANIFEST);
    if !path.is_file() {
        bail!("没有这次纳入的备份：{id}");
    }
    Ok(path)
}

/// 执行中的记录：失败或撤回时按它原样恢复。
#[derive(Default)]
struct Journal {
    id: String,
    dir: PathBuf,
    /// 移走的原位置：(原路径, 备份路径或 None, 链接目标或 None)
    moved: Vec<(PathBuf, Option<PathBuf>, Option<PathBuf>)>,
    /// 来源定义的原值：(来源名, 原定义；None = 本次新建)
    definitions: Vec<(String, Option<Value>)>,
    /// 这次链接上的 (来源, 技能, 账号)
    links: Vec<(String, String, String)>,
    /// 保留本地内容时复制出来的目录与它的内容哈希
    created_dir: Option<(PathBuf, String)>,
}

impl Journal {
    fn to_json(&self, name: &str, mode: &str, done: bool) -> Value {
        json!({
            "id": self.id,
            "name": name,
            "mode": mode,
            "state": if done { "done" } else { "in-progress" },
            "moved": self.moved.iter().map(|(orig, backup, target)| json!({
                "path": orig, "backup": backup, "link_target": target,
            })).collect::<Vec<_>>(),
            "definitions": self.definitions.iter().map(|(n, d)| json!({"collection": n, "previous": d})).collect::<Vec<_>>(),
            "links": self.links.iter().map(|(c, s, a)| json!({"collection": c, "skill": s, "agent": a})).collect::<Vec<_>>(),
            "created_dir": self.created_dir.as_ref().map(|(p, h)| json!({"path": p, "hash": h})),
        })
    }

    fn save(&self, name: &str, mode: &str, done: bool) -> Result<()> {
        std::fs::write(
            self.dir.join(MANIFEST),
            serde_json::to_string_pretty(&self.to_json(name, mode, done))? + "\n",
        )?;
        Ok(())
    }

    fn from_json(dir: PathBuf, value: &Value) -> Journal {
        let path = |v: &Value| v.as_str().map(PathBuf::from);
        Journal {
            id: value["id"].as_str().unwrap_or_default().to_string(),
            dir,
            moved: value["moved"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| {
                    Some((
                        path(&m["path"])?,
                        path(&m["backup"]),
                        path(&m["link_target"]),
                    ))
                })
                .collect(),
            definitions: value["definitions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|d| {
                    let prev = d.get("previous").cloned().filter(|p| !p.is_null());
                    Some((d["collection"].as_str()?.to_string(), prev))
                })
                .collect(),
            links: value["links"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|l| {
                    Some((
                        l["collection"].as_str()?.to_string(),
                        l["skill"].as_str()?.to_string(),
                        l["agent"].as_str()?.to_string(),
                    ))
                })
                .collect(),
            created_dir: value["created_dir"]
                .as_object()
                .and_then(|o| Some((path(o.get("path")?)?, o.get("hash")?.as_str()?.to_string()))),
        }
    }
}

/// 把一个位置移进备份：真实目录整个搬走（跨卷时复制后删除），链接记下目标后删掉。
fn move_away(journal: &mut Journal, index: usize, path: &Path) -> Result<()> {
    let meta =
        std::fs::symlink_metadata(path).with_context(|| format!("读不到 {}", path.display()))?;
    if meta.file_type().is_symlink() {
        let target = crate::platform::read_link(path)?;
        crate::platform::remove_link(path)?;
        journal.moved.push((path.to_path_buf(), None, Some(target)));
        return Ok(());
    }
    let backup = journal
        .dir
        .join(format!("{index}"))
        .join(path.file_name().unwrap_or_default());
    std::fs::create_dir_all(backup.parent().unwrap())?;
    if std::fs::rename(path, &backup).is_err() {
        copy_dir_recursive(path, &backup)?;
        std::fs::remove_dir_all(path)?;
    }
    journal.moved.push((path.to_path_buf(), Some(backup), None));
    Ok(())
}

fn write_definition(ctx: &Context, name: &str, value: Option<Value>) -> Result<()> {
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mut policy = store::policy_from_connection(&tx)?;
    let repos = policy
        .as_object_mut()
        .ok_or_else(|| anyhow!("策略必须是对象"))?
        .entry("skills")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("skills 必须是对象"))?
        .entry("repositories")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("skills.repositories 必须是对象"))?;
    match value {
        Some(def) => {
            repos.insert(name.to_string(), def);
        }
        None => {
            repos.remove(name);
        }
    }
    store::write_policy(&tx, &policy)?;
    tx.commit()?;
    Ok(())
}

fn stored_definition(ctx: &Context, name: &str) -> Result<Option<Value>> {
    let policy = store::effective_policy(ctx)?;
    Ok(policy
        .pointer(&format!("/skills/repositories/{name}"))
        .filter(|d| d.get("_delete").and_then(Value::as_bool) != Some(true))
        .cloned())
}

/// 在锁内调用逐技能的 apply（与界面单独操作同一条路径：预检、链接、记录）。
fn skill_action(
    ctx: &Context,
    run_id: &str,
    collection: &str,
    skill: &str,
    action: &str,
    agent: &str,
) -> Result<()> {
    let sub = Request {
        command: "skills:apply".into(),
        args: vec![
            "--only".into(),
            format!("skill:{collection}/{skill}"),
            "--action".into(),
            action.into(),
            "--agent".into(),
            agent.into(),
        ],
        run_id: run_id.to_string(),
    };
    super::preflight::preflight_apply(ctx, &sub)?;
    super::apply::apply(ctx, &sub)?;
    Ok(())
}

fn collection_update(
    ctx: &Context,
    run_id: &str,
    collection: &str,
    reference: Option<&str>,
) -> Result<()> {
    let mut args = vec![
        "--only".into(),
        format!("skill:{collection}"),
        "--action".into(),
        "update".into(),
    ];
    if let Some(reference) = reference {
        args.push("--ref".into());
        args.push(reference.into());
    }
    let sub = Request {
        command: "skills:apply".into(),
        args,
        run_id: run_id.to_string(),
    };
    super::preflight::preflight_apply(ctx, &sub)?;
    super::apply::apply(ctx, &sub)?;
    Ok(())
}

/// 按记录撤回（失败回滚与用户撤回共用）：解除链接 → 还原定义 → 删掉复制出的目录 → 放回原位置。
fn rewind(ctx: &Context, run_id: &str, journal: &Journal) -> Result<Vec<String>> {
    let mut problems: Vec<String> = Vec::new();
    for (collection, skill, agent) in journal.links.iter().rev() {
        if let Err(err) = skill_action(ctx, run_id, collection, skill, "remove", agent) {
            problems.push(format!("解除 {skill} @ {agent} 失败：{err:#}"));
        }
    }
    for (collection, previous) in journal.definitions.iter().rev() {
        if let Err(err) = write_definition(ctx, collection, previous.clone()) {
            problems.push(format!("还原来源 {collection} 的定义失败：{err:#}"));
        }
    }
    if let Some((dir, hash)) = &journal.created_dir {
        match hash_skill_dir(dir) {
            Ok(now) if now == *hash => {
                let _ = std::fs::remove_dir_all(dir);
            }
            Ok(_) => problems.push(format!("{} 纳入后被改过，保留没删", dir.display())),
            Err(_) => {}
        }
    }
    for (orig, backup, link_target) in journal.moved.iter().rev() {
        if std::fs::symlink_metadata(orig).is_ok() {
            problems.push(format!(
                "{} 已被占用，原内容留在备份里没放回",
                orig.display()
            ));
            continue;
        }
        let restored = match (backup, link_target) {
            (_, Some(target)) => {
                crate::platform::link_dir(target, orig).map_err(anyhow::Error::from)
            }
            (Some(backup), None) => std::fs::rename(backup, orig).or_else(|_| {
                copy_dir_recursive(backup, orig)?;
                std::fs::remove_dir_all(backup).map_err(anyhow::Error::from)
            }),
            (None, None) => Ok(()),
        };
        if let Err(err) = restored {
            problems.push(format!("放回 {} 失败：{err:#}", orig.display()));
        }
    }
    Ok(problems)
}

pub(crate) fn apply(ctx: &Context, req: &Request) -> Result<Value> {
    if let Some(id) = req.value("--undo") {
        return undo(ctx, req, id);
    }
    let name = req.value("--name").ok_or_else(|| anyhow!("需要 --name"))?;
    let expected = req
        .value("--digest")
        .ok_or_else(|| anyhow!("需要 --digest"))?;
    let keep_local = req.flag("--keep-local");
    let mut progress = Progress::new(&req.run_id, &[("check", "核对预览后有没有变化")]);
    progress.start("check", None);
    let plan = local_plan(ctx, name)?;
    let home = ctx.home_dir.clone();
    let local = if keep_local {
        Some(local_destination(ctx, req)?)
    } else {
        None
    };

    // 重算指纹：本地内容、处理方式、目标版本都要与预览一致
    let (mode, collection, reference, target_path, repository, restrict) = if plan.kind == "dedupe"
    {
        if digest(&plan, "dedupe", &Value::Null) != expected {
            bail!("预览后内容变了，没有动手；请重新预览");
        }
        (
            "dedupe",
            plan.managed_by.clone().unwrap_or_default(),
            None,
            None,
            None,
            None,
        )
    } else if keep_local {
        if digest(&plan, "local", &local.as_ref().unwrap().1) != expected {
            bail!("预览后内容变了，没有动手；请重新预览");
        }
        (
            "local",
            super::fork::DEFAULT_COLLECTION.to_string(),
            None,
            None,
            None,
            None,
        )
    } else {
        let repository = plan.source["repository"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| {
                anyhow!("认不出 {name} 的来源仓库，只能保留本地内容纳入（--keep-local）")
            })?;
        let meta = load_metadata(ctx)?;
        let (coll_name, existing) =
            target_collection(&meta.collections, &repository, req.value("--collection"))?;
        let track = match &existing {
            Some(coll) => versioning::track_of(coll)?,
            None => Track::Auto,
        };
        let refs = crate::native_upstream::git_remote_refs(ctx, &repository)
            .map_err(|_| anyhow!("读不到 {repository} 的分支与标签（网络、地址或权限问题）"))?;
        let target = match req.value("--ref") {
            Some(reference) => track::resolve_ref(reference, &refs)?,
            None => track::resolve(&track, &refs)?,
        };
        // 位置以预览给出的为准（来源记录里没有时预览按名字在仓库里找到的）
        let path = req
            .value("--path")
            .or(plan.source["path"].as_str())
            .map(|p| p.trim_matches('/').to_string())
            .filter(|p| !p.is_empty())
            .ok_or_else(|| anyhow!("不知道 {name} 在仓库里的位置；请先预览"))?;
        let expected_fp = fingerprint(
            &repository,
            &path,
            req.value("--ref"),
            &target.commit,
            req.flag("--accept-source"),
        );
        if digest(&plan, "source", &expected_fp) != expected {
            bail!("没有动手：预览后内容或来源的版本变了，或本地内容与来源不一致却没有选择「用来源的版本」；请重新预览");
        }
        let restrict = existing.as_ref().and_then(|c| c.agents.clone());
        (
            "source",
            coll_name,
            req.value("--ref").map(str::to_string),
            Some(path),
            Some(repository),
            restrict,
        )
    };

    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%3f").to_string();
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(40)
        .collect();
    let id = format!("{stamp}-{safe}");
    let steps: &[(&str, &str)] = match mode {
        "dedupe" => &[
            ("check", "核对预览后有没有变化"),
            ("backup", "备份并移走多余的副本"),
        ],
        "local" => &[
            ("check", "核对预览后有没有变化"),
            ("copy", "复制到个人集合目录"),
            ("backup", "备份并移走原来的目录"),
            ("register", "登记来源"),
            ("link", "链接到账号"),
        ],
        _ => &[
            ("check", "核对预览后有没有变化"),
            ("backup", "备份并移走原来的目录"),
            ("register", "登记来源"),
            ("download", "下载来源里的这个技能"),
            ("link", "链接到账号"),
        ],
    };
    let mut progress = {
        drop(progress);
        let mut next = Progress::new(&req.run_id, steps);
        next.start("check", None);
        next
    };
    let mut journal = Journal {
        id: id.clone(),
        dir: ctx.data_dir.join(BACKUP_DIR).join(&id),
        ..Default::default()
    };
    std::fs::create_dir_all(&journal.dir)?;
    journal.save(name, mode, false)?;

    let run = |journal: &mut Journal, progress: &mut Progress| -> Result<()> {
        // Preserve local content in the previewed collection directory.
        if mode == "local" {
            progress.start("copy", None);
            let from = plan
                .locations
                .iter()
                .find(|l| l.hash.is_some())
                .and_then(|l| l.content.clone())
                .ok_or_else(|| anyhow!("找不到 {name} 的内容"))?;
            let dest_root = resolve_loose(&expand_home(&local.as_ref().unwrap().0, &home));
            let current = local_destination(ctx, req)?;
            if current != *local.as_ref().unwrap() {
                bail!("预览后个人集合目录变了，请重新预览");
            }
            let dest = dest_root.join(name);
            if std::fs::symlink_metadata(&dest).is_ok() {
                bail!("{} 已经存在，不覆盖", dest.display());
            }
            std::fs::create_dir_all(&dest_root)?;
            copy_dir_recursive(&from, &dest)?;
            journal.created_dir = Some((dest.clone(), hash_skill_dir(&dest)?));
            journal.save(name, mode, false)?;
        }
        // 移走原位置（先移走，账号目录里的同名入口才空得出来）
        let total = plan.locations.len();
        for (index, loc) in plan.locations.iter().enumerate() {
            progress.start(
                "backup",
                Some(format!(
                    "{}/{total}：{}",
                    index + 1,
                    tilde(&loc.path, &home)
                )),
            );
            move_away(journal, index, &loc.path)?;
            journal.save(name, mode, false)?;
        }
        if mode == "dedupe" {
            return Ok(());
        }
        // 登记来源
        progress.start("register", Some(collection.clone()));
        let previous = stored_definition(ctx, &collection)?;
        let next = match mode {
            "local" => {
                let mut def = previous.clone().unwrap_or_else(|| json!({}));
                let same_dir = def["source_path"].as_str().is_some_and(|raw| {
                    resolve_loose(&expand_home(raw, &home))
                        == resolve_loose(&expand_home(&local.as_ref().unwrap().0, &home))
                });
                if previous.is_some() && !same_dir {
                    bail!("来源 {collection} 已经指向别处");
                }
                if previous.is_none() {
                    def = json!({"source_path": local.as_ref().unwrap().0});
                }
                def
            }
            _ => {
                let path = target_path.clone().unwrap_or_default();
                match previous.clone() {
                    Some(mut def) => {
                        let obj = def
                            .as_object_mut()
                            .ok_or_else(|| anyhow!("来源定义不是对象"))?;
                        if let Some(list) = obj.get_mut("skill_paths").and_then(Value::as_array_mut)
                        {
                            if !list.iter().any(|p| {
                                p.as_str().map(|s| s.trim_matches('/')) == Some(path.as_str())
                                    || p == name
                            }) {
                                list.push(json!(path));
                            }
                        }
                        def
                    }
                    None => json!({"url": repository, "skill_paths": [path]}),
                }
            }
        };
        if previous.as_ref() != Some(&next) {
            journal.definitions.push((collection.clone(), previous));
            journal.save(name, mode, false)?;
            write_definition(ctx, &collection, Some(next))?;
        }
        if mode == "source" {
            progress.start("download", target_path.clone());
            collection_update(ctx, &req.run_id, &collection, reference.as_deref())?;
        }
        // 来源里的技能名以它在仓库里的目录名为准（本地可能换过名字）
        let linked_name = match (&target_path, mode) {
            (Some(path), "source") => path.rsplit('/').next().unwrap_or(name).to_string(),
            _ => name.to_string(),
        };
        let mut agents = link_agents(ctx, restrict.as_ref())?;
        // 用户在纳入时取消勾选的账号不链接（默认全部账号）
        if let Some(chosen) = req.value("--agents") {
            let chosen: Vec<&str> = chosen.split(',').filter(|a| !a.is_empty()).collect();
            if let Some(unknown) = chosen.iter().find(|a| !agents.iter().any(|x| x == *a)) {
                bail!("账号 {unknown} 不能链接这个技能（不存在、已停用或来源限制了账号）");
            }
            agents.retain(|a| chosen.contains(&a.as_str()));
            if agents.is_empty() {
                bail!("至少选一个账号");
            }
        }
        let total = agents.len();
        for (index, agent) in agents.into_iter().enumerate() {
            progress.start("link", Some(format!("{}/{total}：{agent}", index + 1)));
            skill_action(ctx, &req.run_id, &collection, &linked_name, "add", &agent)?;
            journal
                .links
                .push((collection.clone(), linked_name.clone(), agent));
            journal.save(name, mode, false)?;
        }
        Ok(())
    };
    if let Err(err) = run(&mut journal, &mut progress) {
        progress.add_and_start("rollback", "出错了，正在恢复原样");
        let problems = rewind(ctx, &req.run_id, &journal)?;
        let _ = if problems.is_empty() {
            std::fs::remove_dir_all(&journal.dir).map_err(anyhow::Error::from)
        } else {
            journal.save(name, mode, false)
        };
        if problems.is_empty() {
            return Err(err.context("没有纳入，已恢复原样"));
        }
        return Err(err.context(format!(
            "没有纳入；恢复时有问题：{}（备份在 {}）",
            problems.join("；"),
            journal.dir.display()
        )));
    }
    journal.save(name, mode, true)?;
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "skills:adopt",
        Some(name),
        Some(
            &json!({"backup": id, "mode": mode, "collection": collection, "links": journal.links.len(), "removed": journal.moved.len()}),
        ),
    )?;
    tx.commit()?;
    Ok(json!({
        "domain": "skills",
        "action": "adopt",
        "name": name,
        "mode": mode,
        "collection": collection,
        "backup": id,
        "removed": journal.moved.iter().map(|(p, _, _)| tilde(p, &home)).collect::<Vec<_>>(),
        "linked": journal.links.iter().map(|(_, _, a)| a.clone()).collect::<Vec<_>>(),
    }))
}

fn undo(ctx: &Context, req: &Request, id: &str) -> Result<Value> {
    let path = manifest_path(ctx, id)?;
    let value: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    if value["state"] == "undone" {
        bail!("这次纳入已经撤回过了");
    }
    let name = value["name"].as_str().unwrap_or_default().to_string();
    let journal = Journal::from_json(path.parent().unwrap().to_path_buf(), &value);
    // 放回前先确认原位置在解除链接后会是空的：被外部占用就不动手
    let linked_paths: Vec<PathBuf> = {
        let meta = load_metadata(ctx)?;
        journal
            .links
            .iter()
            .filter_map(|(_, skill, agent)| {
                meta.agents.get(agent).map(|a| a.skills_dir.join(skill))
            })
            .collect()
    };
    for (orig, _, _) in &journal.moved {
        if std::fs::symlink_metadata(orig).is_ok() && !linked_paths.contains(orig) {
            bail!("{} 已被别的内容占用，没有撤回", orig.display());
        }
    }
    let problems = rewind(ctx, &req.run_id, &journal)?;
    let mut done = value.clone();
    done["state"] = json!(if problems.is_empty() {
        "undone"
    } else {
        "undo-partial"
    });
    std::fs::write(&path, serde_json::to_string_pretty(&done)? + "\n")?;
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "skills:adopt-undo",
        Some(&name),
        Some(&json!({"backup": id, "problems": problems})),
    )?;
    tx.commit()?;
    if !problems.is_empty() {
        bail!("撤回没有完全成功：{}", problems.join("；"));
    }
    Ok(json!({"domain": "skills", "action": "adopt-undo", "name": name, "backup": id}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    #[test]
    fn 同一个仓库的不同写法认作一个() {
        assert!(same_repo(
            "https://github.com/Owner/Repo.git",
            "https://github.com/owner/repo"
        ));
        assert!(same_repo(
            "git@github.com:owner/repo.git",
            "https://github.com/owner/repo/"
        ));
        assert!(!same_repo(
            "https://github.com/owner/repo",
            "https://github.com/owner/repo2"
        ));
    }

    #[test]
    fn 新来源名取仓库名_重名加序号() {
        let mut colls: BTreeMap<String, CollectionDef> = BTreeMap::new();
        let (name, existing) =
            target_collection(&colls, "https://github.com/tt-a1i/archify.git", None).unwrap();
        assert_eq!(name, "archify");
        assert!(existing.is_none());
        colls.insert(
            "archify".into(),
            CollectionDef {
                name: "archify".into(),
                url: Some("https://example.invalid/other.git".into()),
                source_path: None,
                skill_paths: None,
                agents: None,
                disabled: None,
                enabled: true,
                deleted: false,
                raw: Map::new(),
            },
        );
        let (name, _) =
            target_collection(&colls, "https://github.com/tt-a1i/archify.git", None).unwrap();
        assert_eq!(name, "archify-2");
        let (name, existing) =
            target_collection(&colls, "https://example.invalid/other", None).unwrap();
        assert_eq!(name, "archify");
        assert!(existing.is_some());
    }
}
