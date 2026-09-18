//! profiles 域真实写入：为一个实例建好独立配置目录并写入它的那份配置文件。
//!
//! 写入面只有 `<配置目录>/settings.json`（Claude）或 `<配置目录>/config.toml`（Codex），
//! 这个文件由本工具全权拥有：目标已存在且不是本工具最后写入的那份时 fail-closed，
//! 需要显式 `--allow-takeover` 才替换。全局 `~/.claude` 与 `~/.codex` 全程不碰。
//!
//! 事务顺序：preflight（零写）→ 取全局锁（调用方）→ 建目录 → 原子换入文件 →
//! 写 binding 与审计 → 提交；提交失败即回滚文件到写前状态。

use anyhow::{anyhow, bail, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use super::model::{self, ProfileDef};
use super::persistence::{load_bindings_from, write_binding, Binding};
use crate::native::{Context, Request};
use crate::native_mcp::model::sha256_hex_bytes;
use crate::store;

fn parse_only(req: &Request) -> Result<String> {
    let only = req
        .value("--only")
        .ok_or_else(|| anyhow!("缺少 --only 参数（形如 --only profiles:<实例名>）"))?;
    let id = only.strip_prefix("profiles:").unwrap_or(only);
    if !model::valid_profile_id(id) {
        bail!("实例名只能用小写字母、数字与连字符");
    }
    Ok(id.to_string())
}

fn find_def(ctx: &Context, id: &str) -> Result<ProfileDef> {
    let policy = store::effective_policy(ctx)?;
    for parsed in model::load_definitions(&policy) {
        match parsed {
            Ok(def) if def.id == id => return Ok(def),
            Ok(_) => {}
            // 其它实例定义损坏不影响本次目标；本实例自身损坏会在下面的精确匹配里暴露
            Err(_) => {}
        }
    }
    // 精确定位：目标实例可能就是那条损坏定义
    let policy = store::effective_policy(ctx)?;
    if let Some(raw) = policy
        .get("agents")
        .and_then(|v| v.get("definitions"))
        .and_then(Value::as_object)
        .and_then(|defs| defs.get(id))
    {
        model::parse_definition(id, raw, true)?;
    }
    bail!("未知实例: {}", id)
}

/// 锁前预检：参数与状态校验，全程零写。
pub(crate) fn preflight_apply(ctx: &Context, req: &Request) -> Result<()> {
    if req.flag("--dry-run") {
        bail!("profiles:apply 已退役 --dry-run，拒绝该参数");
    }
    for arg in &req.args {
        if !matches!(arg.as_str(), "--only" | "--allow-takeover") && arg.starts_with("--") {
            bail!("profiles:apply 不支持参数 '{}'", arg);
        }
    }
    let id = parse_only(req)?;
    let def = find_def(ctx, &id)?;
    if !def.enabled {
        bail!("[{}] 实例已停用，拒绝写入", id);
    }
    let dir = model::validate_config_dir(&def, &ctx.home_dir, &ctx.data_dir)?;
    // 渲染一次：变量缺失、来源结构非法在取锁前就暴露
    let rendered = model::render_all(
        ctx,
        &def,
        &|file| std::fs::read(dir.join(file)).ok(),
        &|name| std::env::var(name).ok(),
    )?;

    let allow = req.flag("--allow-takeover");
    for target in &rendered {
        let path = dir.join(&target.file);
        if std::fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            bail!("[{}] {} 是符号链接，拒绝写入", id, target.file);
        }
        if path.exists() && !path.is_file() {
            bail!("[{}] {} 被目录占用，拒绝写入", id, target.file);
        }
        check_takeover(ctx, &def, &target.file, &path, allow)?;
    }
    Ok(())
}

/// 目标已存在但不是本工具最后写入的那份时，必须显式接管。
fn check_takeover(
    ctx: &Context,
    def: &ProfileDef,
    file: &str,
    path: &Path,
    allow: bool,
) -> Result<bool> {
    if !path.is_file() {
        return Ok(false);
    }
    let actual = sha256_hex_bytes(&std::fs::read(path)?);
    let known = match store::open_readonly(ctx)? {
        Some(conn) => load_bindings_from(&conn)?
            .get(&(def.id.clone(), file.to_string()))
            .map(|b| b.applied_digest.clone()),
        None => None,
    };
    if known.as_deref() == Some(actual.as_str()) {
        return Ok(false);
    }
    if !allow {
        bail!(
            "[{}] {} 已存在且不是本工具写入的那份，拒绝覆盖；确认要接管请显式选择接管",
            def.id,
            file
        );
    }
    Ok(true)
}

/// 原子换入：同目录临时文件 + rename，权限 0600。
fn atomic_write(target: &Path, content: &[u8]) -> Result<()> {
    let dir = target
        .parent()
        .ok_or_else(|| anyhow!("目标路径没有父目录"))?;
    let tmp = dir.join(format!(
        ".{}.am-tmp",
        target
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "profile".to_string())
    ));
    std::fs::write(&tmp, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, target)?;
    Ok(())
}

fn ensure_dir(path: &PathBuf) -> Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub(crate) fn apply(ctx: &Context, req: &Request) -> Result<Value> {
    preflight_apply(ctx, req)?;
    let id = parse_only(req)?;
    let def = find_def(ctx, &id)?;
    let dir = model::validate_config_dir(&def, &ctx.home_dir, &ctx.data_dir)?;
    let allow = req.flag("--allow-takeover");
    let rendered = model::render_all(
        ctx,
        &def,
        &|file| std::fs::read(dir.join(file)).ok(),
        &|name| std::env::var(name).ok(),
    )?;

    let mut takeover = false;
    for target in &rendered {
        takeover |= check_takeover(ctx, &def, &target.file, &dir.join(&target.file), allow)?;
    }

    ensure_dir(&dir)?;
    // 写前留存每个文件的原状，任一步失败就整体还原，不留半套配置
    let mut restore: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
    let mut written: Vec<(String, String)> = Vec::new();
    let rollback = |restore: &[(PathBuf, Option<Vec<u8>>)]| {
        for (path, previous) in restore {
            match previous {
                Some(bytes) => {
                    let _ = std::fs::write(path, bytes);
                }
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    };

    for target in &rendered {
        let path = dir.join(&target.file);
        let previous = if path.is_file() {
            match std::fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(err) => {
                    rollback(&restore);
                    return Err(err.into());
                }
            }
        } else {
            None
        };
        restore.push((path.clone(), previous));
        if let Err(err) = atomic_write(&path, &target.content) {
            rollback(&restore);
            return Err(err);
        }
        written.push((target.file.clone(), sha256_hex_bytes(&target.content)));
    }

    let result = (|| -> Result<()> {
        let mut conn = store::open_write(ctx)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (file, digest) in &written {
            write_binding(
                &tx,
                &Binding {
                    profile_id: def.id.clone(),
                    file: file.clone(),
                    applied_digest: digest.clone(),
                    updated_at: chrono::Utc::now()
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                },
            )?;
        }
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "profiles:apply",
            Some(&def.id),
            Some(&json!({
                "action": "apply",
                "kind": def.kind.as_str(),
                "takeover": takeover,
                "files": written.iter().map(|(file, _)| file.clone()).collect::<Vec<_>>(),
            })),
        )?;
        tx.commit()?;
        Ok(())
    })();

    if let Err(err) = result {
        rollback(&restore);
        return Err(err);
    }

    Ok(json!({
        "domain": "profiles",
        "steps": [{"name": format!("profiles:apply {}", def.id), "ok": true, "exit_code": 0}],
        "skipped": [],
    }))
}
