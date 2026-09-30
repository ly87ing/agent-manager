//! git 来源的仓库镜像：只下载指定目录、按跟踪方式或指定版本切换、记录当前版本。
//!
//! 镜像是本工具自己的只读副本（`repositories/<来源名>`），本地不产生提交：
//! - 新下载用部分克隆（`--filter=tree:0 --no-checkout`），再按 `skill_paths`、规则目录与
//!   `aliases/` 做稀疏检出，其余目录不下载；`skill_paths` 缺省（整个仓库找技能）时完整检出；
//! - 切换版本：取远端引用清单 → 按显式版本或跟踪方式解析目标 → `fetch` → 分离检出到目标提交；
//! - 检出后把「当前版本」与全仓库的技能清单写进镜像根目录的 `.agent-manager-state.json`，
//!   status 读它（不起进程、不访问网络）；镜像被外部改动过时以 `.git/HEAD` 为准。

use anyhow::{anyhow, bail, Context as _, Result};
use serde_json::{json, Value};
use std::path::Path;

use super::track::{self, RemoteRefs, Target, Track};
use super::CollectionDef;
use crate::native::Context;

pub(crate) const STATE_FILE: &str = ".agent-manager-state.json";
const STATE_LIMIT: u64 = 4 * 1024 * 1024;

/// 在镜像目录里执行 git；不交互，失败时带上 git 的错误输出。
pub(crate) fn git_in(repo_dir: &Path, args: &[&str], coll_name: &str) -> Result<String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(repo_dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .with_context(|| format!("git {} 启动失败: {}", args[0], repo_dir.display()))?;
    if !out.status.success() {
        let detail = String::from_utf8_lossy(&out.stderr);
        let detail = detail.trim();
        bail!(
            "[{}] git {} 失败{}",
            coll_name,
            args[0],
            if detail.is_empty() {
                String::new()
            } else {
                format!("：{detail}")
            }
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// 来源定义里的跟踪方式；写错时报错，不静默当成自动。
pub(crate) fn track_of(coll: &CollectionDef) -> Result<Track> {
    Track::parse(coll.raw.get("track")).map_err(|err| anyhow!("[{}] {err}", coll.name))
}

/// 稀疏检出的目录：技能目录 + 规则目录（默认 `rules`）+ `aliases`。技能目录缺省时为 None（完整检出）。
pub(crate) fn sparse_patterns(coll: &CollectionDef) -> Result<Option<Vec<String>>> {
    let Some(paths) = coll.skill_paths.as_ref().filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    let rules = coll
        .raw
        .get("rules_path")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .unwrap_or("rules");
    let mut out: Vec<String> = Vec::new();
    for raw in paths.iter().map(String::as_str).chain([rules, "aliases"]) {
        let rel = raw.trim().trim_matches('/');
        if rel.is_empty() || rel == "." {
            // 指向仓库根：整个仓库都要
            return Ok(None);
        }
        if rel.starts_with('-')
            || rel.split('/').any(|part| part == ".." || part.is_empty())
            || rel
                .chars()
                .any(|c| matches!(c, '*' | '?' | '[' | '\\' | '!' | '#' | '\n'))
        {
            bail!("[{}] 目录 {raw} 不合法", coll.name);
        }
        let pattern = format!("/{rel}/");
        if !out.contains(&pattern) {
            out.push(pattern);
        }
    }
    Ok(Some(out))
}

/// `.git/HEAD` 指向的提交；只读文件，不起进程。符号引用在松散引用或 packed-refs 里找。
pub(crate) fn head_commit(repo_dir: &Path) -> Option<String> {
    let git = repo_dir.join(".git");
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    let Some(reference) = head.strip_prefix("ref: ") else {
        return track::is_sha(head).then(|| head.to_string());
    };
    if let Ok(sha) = std::fs::read_to_string(git.join(reference)) {
        let sha = sha.trim();
        return track::is_sha(sha).then(|| sha.to_string());
    }
    let packed = std::fs::read_to_string(git.join("packed-refs")).ok()?;
    packed.lines().find_map(|line| {
        let (sha, name) = line.split_once(' ')?;
        (name.trim() == reference && track::is_sha(sha)).then(|| sha.to_string())
    })
}

fn read_state(repo_dir: &Path) -> Option<Value> {
    let path = repo_dir.join(STATE_FILE);
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > STATE_LIMIT {
        return None;
    }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// 镜像当前所在版本：`{commit, kind, name, label}`；镜像不存在为 null。
/// 记录与 `.git/HEAD` 对不上（被外部改过）时只报提交。
pub(crate) fn local_version(repo_dir: &Path) -> Value {
    let Some(commit) = head_commit(repo_dir) else {
        return Value::Null;
    };
    if let Some(state) = read_state(repo_dir) {
        if state.get("commit").and_then(Value::as_str) == Some(commit.as_str()) {
            return json!({
                "commit": commit,
                "kind": state.get("kind").cloned().unwrap_or(Value::Null),
                "name": state.get("name").cloned().unwrap_or(Value::Null),
                "label": state.get("label").cloned().unwrap_or(Value::Null),
            });
        }
    }
    let short: String = commit.chars().take(7).collect();
    json!({"commit": commit, "kind": "commit", "name": commit, "label": short})
}

/// 全仓库的技能（名 → 仓库内路径），来自检出时的记录；记录与当前提交对不上时为 None。
pub(crate) fn recorded_available(repo_dir: &Path) -> Option<Vec<(String, String)>> {
    let state = read_state(repo_dir)?;
    if state.get("commit").and_then(Value::as_str) != head_commit(repo_dir).as_deref() {
        return None;
    }
    let list = state.get("available")?.as_array()?;
    Some(
        list.iter()
            .filter_map(|item| {
                Some((
                    item.get("name")?.as_str()?.to_string(),
                    item.get("path")?.as_str()?.to_string(),
                ))
            })
            .collect(),
    )
}

/// 目标提交里所有 `SKILL.md` 所在目录（与 `available_repo_skills` 同一规则：
/// 目录名为技能名，重名取先出现的，隐藏目录与依赖目录跳过）。
fn available_at_head(repo_dir: &Path, coll_name: &str) -> Result<Vec<Value>> {
    let listing = git_in(
        repo_dir,
        &["ls-tree", "-r", "--name-only", "HEAD"],
        coll_name,
    )?;
    let mut out: Vec<Value> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for file in listing.lines() {
        let Some(dir) = file
            .strip_suffix("/SKILL.md")
            .or((file == "SKILL.md").then_some(""))
        else {
            continue;
        };
        let parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
        if parts.len() > 3
            || parts
                .iter()
                .any(|p| p.starts_with('.') || super::EXCLUDED_DIRS.contains(p))
        {
            continue;
        }
        let name = match parts.last() {
            Some(name) => name.to_string(),
            None => repo_dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
        };
        if names.contains(&name) {
            continue;
        }
        names.push(name.clone());
        out.push(json!({"name": name, "path": dir}));
    }
    Ok(out)
}

/// 把镜像更新到目标版本：显式 `reference`（标签、分支或提交）优先，否则按跟踪方式。
/// 返回 `{from, to}`（to 为目标版本）；网络、git 或目录问题一律报错，不静默成功。
pub(crate) fn update_repo(
    ctx: &Context,
    coll: &CollectionDef,
    url: &str,
    repo_dir: &Path,
    reference: Option<&str>,
) -> Result<Value> {
    let refs: RemoteRefs = crate::native_upstream::git_remote_refs(ctx, url).map_err(|_| {
        anyhow!(
            "[{}] 读不到远端的分支与标签（网络、地址或权限问题）",
            coll.name
        )
    })?;
    let target: Target = match reference {
        Some(reference) => track::resolve_ref(reference, &refs)?,
        None => track::resolve(&track_of(coll)?, &refs)?,
    };
    let patterns = sparse_patterns(coll)?;
    let before = local_version(repo_dir);

    if !repo_dir.join(".git").is_dir() {
        std::fs::create_dir_all(super::repo_base_dir(ctx))?;
        let status = std::process::Command::new("git")
            .args([
                "clone",
                "--quiet",
                "--filter=tree:0",
                "--no-checkout",
                "--",
                url,
            ])
            .arg(repo_dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .status()
            .with_context(|| "git clone 启动失败")?;
        if !status.success() {
            bail!("[{}] git clone 失败", coll.name);
        }
        std::fs::write(
            repo_dir.join(".agent-manager-owned"),
            serde_json::to_string_pretty(&json!({
                "managed_by": "agent-manager",
                "repository": coll.name
            }))? + "\n",
        )?;
    }

    match &patterns {
        Some(patterns) => {
            let mut args = vec!["sparse-checkout", "set", "--no-cone"];
            args.extend(patterns.iter().map(String::as_str));
            git_in(repo_dir, &args, &coll.name)?;
        }
        None => {
            // 之前是稀疏检出、现在要整个仓库：关掉稀疏
            let sparse = git_in(
                repo_dir,
                &["config", "--get", "core.sparseCheckout"],
                &coll.name,
            )
            .unwrap_or_default();
            if sparse.trim() == "true" {
                git_in(repo_dir, &["sparse-checkout", "disable"], &coll.name)?;
            }
        }
    }
    // 镜像只读：远端就是唯一真相，历史被改写（force push）时照样对齐
    git_in(
        repo_dir,
        &[
            "fetch",
            "--quiet",
            "--force",
            "--tags",
            "origin",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
        &coll.name,
    )?;
    git_in(
        repo_dir,
        &[
            "-c",
            "advice.detachedHead=false",
            "checkout",
            "--quiet",
            "--force",
            "--detach",
            &target.commit,
        ],
        &coll.name,
    )?;
    let commit =
        head_commit(repo_dir).ok_or_else(|| anyhow!("[{}] 检出后读不到当前提交", coll.name))?;
    let target = Target {
        commit: commit.clone(),
        ..target
    };
    let available = available_at_head(repo_dir, &coll.name)?;
    std::fs::write(
        repo_dir.join(STATE_FILE),
        serde_json::to_string_pretty(&json!({
            "commit": commit,
            "kind": target.kind,
            "name": target.name,
            "label": target.label(),
            "available": available,
        }))? + "\n",
    )?;
    Ok(json!({"from": before, "to": target.to_json()}))
}

/// `skills:versions`：列出某个 git 来源可选的版本，并给出按跟踪方式解析的目标与当前版本。
/// 只读网络；不写库、不动镜像。
pub(crate) fn versions(ctx: &Context, coll: &CollectionDef, repo_dir: &Path) -> Result<Value> {
    let url = coll
        .url
        .as_deref()
        .filter(|u| !u.is_empty())
        .ok_or_else(|| anyhow!("[{}] 不是 git 来源，没有版本", coll.name))?;
    let track = track_of(coll)?;
    let refs = crate::native_upstream::git_remote_refs(ctx, url).map_err(|_| {
        anyhow!(
            "[{}] 读不到远端的分支与标签（网络、地址或权限问题）",
            coll.name
        )
    })?;
    let latest = track::resolve(&track, &refs);
    let effective = match (&track, &latest) {
        (Track::Auto, Ok(target)) if target.kind == "tag" => "tag",
        (Track::Auto, _) => "default",
        _ => "",
    };
    let mut listing = track::listing(&refs, 200);
    let obj = listing.as_object_mut().expect("listing 是对象");
    obj.insert("collection".into(), json!(coll.name));
    obj.insert("track".into(), track.to_json());
    if !effective.is_empty() {
        obj.insert("auto_resolved".into(), json!(effective));
    }
    obj.insert(
        "latest".into(),
        match latest {
            Ok(target) => target.to_json(),
            Err(err) => json!({"error": err.to_string()}),
        },
    );
    obj.insert("current".into(), local_version(repo_dir));
    Ok(listing)
}
