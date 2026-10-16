//! 「改成我的版本」：把某个集合里的技能复制到用户自己的目录，登记为用户的本机目录集合，
//! 并记下它来自哪个集合的哪个版本。之后用户的版本与原集合互不影响；原集合出新版时，
//! status 只提示「上游有新版」，不会改用户的文件。
//!
//! This command copies and registers only. The interface switches each selected
//! assistant with a guarded atomic `skills:apply --replace-from` operation.

use anyhow::{anyhow, bail, Context as _, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Map, Value};
use std::path::Path;

use super::paths::{
    collection_source_root, expand_home, personal_source_path, resolve_loose,
    validate_path_identifier,
};
use super::persistence::load_bindings_readonly;
use super::snapshot::{
    copy_dir_recursive, discover_repo_skills, hash_skill_dir, verify_skill_version,
};
use super::status::{load_metadata, DomainMetadata};
use super::CollectionDef;
use crate::native::{Context, Request};
use crate::store;

pub(super) const DEFAULT_COLLECTION: &str = "my-skills";

/// 复制出来的文件要能改：快照里的文件是只读的。
#[cfg_attr(not(unix), allow(unused_variables))]
pub(super) fn make_writable(dir: &Path) -> Result<()> {
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
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let meta = std::fs::symlink_metadata(&path)?;
            // Never follow a link or another reparse point into source files.
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                continue;
            }
            if meta.is_dir() {
                make_writable(&path)?;
            } else if meta.is_file() && meta.permissions().readonly() {
                let mut permissions = meta.permissions();
                permissions.set_readonly(false);
                std::fs::set_permissions(&path, permissions)?;
            }
        }
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn making_a_copy_writable_does_not_follow_directory_reparse_points() {
        let root = std::env::temp_dir().join(format!("am-fork-attributes-{}", store::uuid_v4()));
        let copy = root.join("copy");
        let outside = root.join("outside");
        std::fs::create_dir_all(copy.join("nested")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let own_file = copy.join("nested/file.txt");
        let outside_file = outside.join("source.txt");
        for file in [&own_file, &outside_file] {
            std::fs::write(file, b"original").unwrap();
            let mut permissions = std::fs::metadata(file).unwrap().permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(file, permissions).unwrap();
        }
        let link = copy.join("linked-directory");
        junction::create(&outside, &link).unwrap();
        make_writable(&copy).unwrap();
        std::fs::write(&own_file, b"edited copy").unwrap();
        assert_eq!(std::fs::read(&own_file).unwrap(), b"edited copy");
        assert!(std::fs::metadata(&outside_file)
            .unwrap()
            .permissions()
            .readonly());
        assert_eq!(std::fs::read(&outside_file).unwrap(), b"original");
        junction::delete(&link).unwrap();
        let mut permissions = std::fs::metadata(&outside_file).unwrap().permissions();
        permissions.set_readonly(false);
        std::fs::set_permissions(&outside_file, permissions).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}

fn destination(
    ctx: &Context,
    req: &Request,
    meta: &DomainMetadata,
) -> Result<(String, String, String, String)> {
    if req.flag("--dry-run") {
        bail!("skills:fork 不支持 --dry-run，未复制任何内容");
    }
    let only = req
        .value("--only")
        .ok_or_else(|| anyhow!("skills:fork 需要 --only skill:<集合>/<技能>"))?;
    let (coll_name, skill) = only
        .strip_prefix("skill:")
        .and_then(|rest| rest.split_once('/'))
        .ok_or_else(|| anyhow!("--only 要写成 skill:<集合>/<技能>"))?;
    validate_path_identifier(coll_name, "集合名称")?;
    validate_path_identifier(skill, "技能名")?;
    if skill.contains("..") || skill.starts_with('.') {
        bail!("技能名不合法: {skill}");
    }
    let target = req.value("--collection").unwrap_or(DEFAULT_COLLECTION);
    if target == coll_name {
        bail!("要放进另一个集合，不能是 {coll_name} 自己");
    }
    let dest_raw = personal_source_path(ctx, meta, target, req.value("--dest"))?;
    let dest_root = resolve_loose(&expand_home(&dest_raw, &ctx.home_dir));
    let dest = dest_root.join(skill);
    if dest.exists() || std::fs::symlink_metadata(&dest).is_ok() {
        bail!("{} 已经存在，不覆盖；换个目录或先把它移走", dest.display());
    }

    if !meta
        .collections
        .get(coll_name)
        .is_some_and(|coll| !coll.deleted)
    {
        bail!("没有集合 {coll_name}");
    }
    Ok((coll_name.into(), skill.into(), target.into(), dest_raw))
}

pub(super) fn preflight(ctx: &Context, req: &Request) -> Result<()> {
    destination(ctx, req, &load_metadata(ctx)?)?;
    Ok(())
}

pub(crate) fn fork(ctx: &Context, req: &Request) -> Result<Value> {
    let meta = load_metadata(ctx)?;
    let (coll_name, skill, target, dest_raw) = destination(ctx, req, &meta)?;
    let origin = &meta.collections[&coll_name];
    let dest_root = resolve_loose(&expand_home(&dest_raw, &ctx.home_dir));
    let dest = dest_root.join(&skill);
    let real_root = crate::native_profiles::paths::resolved(&dest_root)?;

    // Git assignments use snapshots; local assignments follow the actual source.
    let bindings = load_bindings_readonly(ctx)?;
    let in_use = bindings
        .iter()
        .find(|((c, s, _), b)| {
            origin.source_path.is_none()
                && c == &coll_name
                && s == &skill
                && b.assigned
                && b.enabled
        })
        .map(|(_, b)| b.version.clone());
    let (source, version) = match in_use {
        Some(version) => (verify_skill_version(&ctx.data_dir, &version)?, version),
        None => {
            let root = collection_source_root(ctx, origin);
            let (found, _) = discover_repo_skills(&root, &origin.raw)?;
            let path = found
                .get(&skill)
                .cloned()
                .ok_or_else(|| anyhow!("集合 {coll_name} 里没有技能 {skill}"))?;
            let version = hash_skill_dir(&path)?;
            (path, version)
        }
    };

    std::fs::create_dir_all(&dest_root)
        .with_context(|| format!("无法创建目录 {}", dest_root.display()))?;
    std::fs::create_dir(&dest)
        .with_context(|| format!("无法创建技能目录 {}（不覆盖已有目录）", dest.display()))?;
    let registered = (|| -> Result<()> {
        copy_dir_recursive(&source, &dest)?;
        make_writable(&dest)?;
        let mut conn = store::open_write(ctx)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut policy = store::policy_from_connection(&tx)?;
        // Definition writers use their own transactions; recheck the copied
        // destination against the current policy while registration is locked.
        let current_meta = DomainMetadata {
            collections: super::status::load_collections(ctx, &policy)?,
            agents: meta.agents.clone(),
        };
        personal_source_path(ctx, &current_meta, &target, Some(&dest_raw))?;
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
        let cleanup = (|| -> Result<()> {
            crate::native_profiles::paths::validate_shell_target(ctx, &dest, true)?;
            if crate::native_profiles::paths::resolved(&dest_root)? != real_root
                || super::paths::is_link_entry(&dest)
            {
                bail!("复制目录已被外部修改，保留 {}", dest.display());
            }
            std::fs::remove_dir_all(&dest)?;
            Ok(())
        })();
        if let Err(cleanup) = cleanup {
            return Err(err.context(format!("复制失败后的清理未完成: {cleanup}")));
        }
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
