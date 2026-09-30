//! 「改成我的版本」：把某个集合里的技能复制到用户自己的目录，登记为用户的本机目录集合，
//! 并记下它来自哪个集合的哪个版本。之后用户的版本与原集合互不影响；原集合出新版时，
//! status 只提示「上游有新版」，不会改用户的文件。
//!
//! 这里只做复制与登记；各账号从原版本换到用户版本由界面用已有的 `skills:apply`
//! （先 remove 原分配、再 add 用户的）逐个账号完成，每一步照常核对。

use anyhow::{anyhow, bail, Context as _, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Map, Value};
use std::path::Path;

use super::paths::{collection_source_root, expand_home, resolve_loose};
use super::persistence::load_bindings_readonly;
use super::snapshot::{
    copy_dir_recursive, discover_repo_skills, hash_skill_dir, verify_skill_version,
};
use super::status::{load_metadata, DomainMetadata};
use super::CollectionDef;
use crate::native::{Context, Request};
use crate::store;

pub(super) const DEFAULT_DEST: &str = "~/my-skills";
pub(super) const DEFAULT_COLLECTION: &str = "my-skills";

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
}

/// 复制出来的文件要能改：快照里的文件是只读的。
#[cfg_attr(not(unix), allow(unused_variables))]
fn make_writable(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let meta = std::fs::symlink_metadata(&path)?;
            if meta.is_dir() {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
                make_writable(&path)?;
            } else if meta.is_file() {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
            }
        }
    }
    Ok(())
}

pub(crate) fn fork(ctx: &Context, req: &Request) -> Result<Value> {
    let only = req
        .value("--only")
        .ok_or_else(|| anyhow!("skills:fork 需要 --only skill:<集合>/<技能>"))?;
    let (coll_name, skill) = only
        .strip_prefix("skill:")
        .and_then(|rest| rest.split_once('/'))
        .ok_or_else(|| anyhow!("--only 要写成 skill:<集合>/<技能>"))?;
    if skill.is_empty() || skill.contains('/') || skill.contains("..") || skill.starts_with('.') {
        bail!("技能名不合法: {skill}");
    }
    let target = req.value("--collection").unwrap_or(DEFAULT_COLLECTION);
    if !valid_name(target) {
        bail!("集合名只能用小写字母、数字和连字符: {target}");
    }
    if target == coll_name {
        bail!("要放进另一个集合，不能是 {coll_name} 自己");
    }
    let dest_raw = req.value("--dest").unwrap_or(DEFAULT_DEST);
    if !(std::path::Path::new(dest_raw).is_absolute() || dest_raw.starts_with("~/")) {
        bail!("目录要写绝对路径或以 ~/ 开头: {dest_raw}");
    }
    let dest_root = resolve_loose(&expand_home(dest_raw, &ctx.home_dir));
    let dest = dest_root.join(skill);
    if dest.exists() || std::fs::symlink_metadata(&dest).is_ok() {
        bail!("{} 已经存在，不覆盖；换个目录或先把它移走", dest.display());
    }

    let meta = load_metadata(ctx)?;
    let origin = meta
        .collections
        .get(coll_name)
        .filter(|coll| !coll.deleted)
        .ok_or_else(|| anyhow!("没有集合 {coll_name}"))?;
    if let Some(existing) = meta.collections.get(target).filter(|coll| !coll.deleted) {
        let same_dir = existing
            .source_path
            .as_deref()
            .map(|raw| resolve_loose(&expand_home(raw, &ctx.home_dir)) == dest_root)
            .unwrap_or(false);
        if !same_dir {
            bail!("集合 {target} 已经指向别处；换个集合名或目录");
        }
    }

    // 复制用户现在用着的那一版；没有账号在用就取来源里的当前版本
    let bindings = load_bindings_readonly(ctx)?;
    let in_use = bindings
        .iter()
        .find(|((c, s, _), b)| c == coll_name && s == skill && b.assigned && b.enabled)
        .map(|(_, b)| b.version.clone());
    let (source, version) = match in_use {
        Some(version) => (verify_skill_version(&ctx.data_dir, &version)?, version),
        None => {
            let root = collection_source_root(ctx, origin);
            let (found, _) = discover_repo_skills(&root, &origin.raw)?;
            let path = found
                .get(skill)
                .cloned()
                .ok_or_else(|| anyhow!("集合 {coll_name} 里没有技能 {skill}"))?;
            let version = hash_skill_dir(&path)?;
            (path, version)
        }
    };

    std::fs::create_dir_all(&dest_root)
        .with_context(|| format!("无法创建目录 {}", dest_root.display()))?;
    copy_dir_recursive(&source, &dest)?;
    let registered = (|| -> Result<()> {
        make_writable(&dest)?;
        let mut conn = store::open_write(ctx)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
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
        let def = repos.entry(target.to_string()).or_insert_with(|| json!({}));
        let def = def
            .as_object_mut()
            .ok_or_else(|| anyhow!("集合 {target} 的定义不是对象"))?;
        if def.get("_delete").and_then(Value::as_bool) == Some(true)
            || def.get("source_path").is_none()
        {
            // 新登记（或之前删过）：一个指向用户目录的本机集合，里面有什么就用什么
            def.clear();
            def.insert("source_path".into(), json!(dest_raw));
        }
        let forked = def.entry("forked_from").or_insert_with(|| json!({}));
        if let Some(forked) = forked.as_object_mut() {
            forked.insert(
                skill.to_string(),
                json!({ "collection": coll_name, "version": version }),
            );
        }
        store::write_policy(&tx, &policy)?;
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "skills:fork",
            Some(&format!("{coll_name}/{skill}")),
            Some(
                &json!({ "collection": target, "path": dest.display().to_string(), "version": version }),
            ),
        )?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = registered {
        // 登记没成功：刚复制出来的目录也撤掉，不留半截
        let _ = std::fs::remove_dir_all(&dest);
        return Err(err);
    }
    Ok(json!({
        "domain": "skills",
        "collection": target,
        "skill": skill,
        "path": dest.display().to_string(),
        "from": { "collection": coll_name, "version": version },
    }))
}

/// 用户版本的来历：来自哪个集合的哪个版本，以及那个集合现在是否有更新的版本（只读）。
pub(crate) fn origin_json(
    ctx: &Context,
    meta: &DomainMetadata,
    coll: &CollectionDef,
    skill: &str,
) -> Value {
    let Some(entry) = coll
        .raw
        .get("forked_from")
        .and_then(|map| map.get(skill))
        .and_then(Value::as_object)
    else {
        return Value::Null;
    };
    let origin = entry
        .get("collection")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let version = entry
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let upstream = meta
        .collections
        .get(origin)
        .filter(|coll| !coll.deleted)
        .and_then(|coll| {
            let root = collection_source_root(ctx, coll);
            let (found, _) = discover_repo_skills(&root, &coll.raw).ok()?;
            hash_skill_dir(found.get(skill)?).ok()
        });
    let mut out = Map::new();
    out.insert("collection".into(), json!(origin));
    out.insert("version".into(), json!(version));
    out.insert("upstream_version".into(), json!(upstream));
    out.insert(
        "upstream_changed".into(),
        json!(upstream.as_deref().is_some_and(|up| up != version)),
    );
    Value::Object(out)
}
