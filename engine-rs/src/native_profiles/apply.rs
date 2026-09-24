//! profiles 域真实写入：为一个实例建好独立配置目录并写入它的那份配置文件。
//!
//! 写入面只有 `<配置目录>/settings.json`（Claude，只改实例定义声明的 env 变量与 model）
//! 或 `<配置目录>/config.toml`（Codex，只改 model、model_provider 与本工具那张接入表）。以实际文件为准：
//! 调用方必须带 status 给出的 `--base`，实际与之不符时拒绝（「已在外部修改」），文件与库都
//! 不动；不再有「接管 = 用库覆盖」。目标是符号链接时写到最终目标文件，链接本身不变。
//! 全局 `~/.claude` 与 `~/.codex` 全程不碰。
//!
//! `profiles:adopt` 反方向：把实际文件里的值写回实例定义（只改库，不动文件），同样按
//! `--base` 确认用户看到的就是这份实际内容；凭据不以明文进库。
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

fn parse_base(req: &Request) -> Result<String> {
    let base = req
        .value("--base")
        .ok_or_else(|| anyhow!("缺少 --base（取 profiles:status 给出的写入基准）"))?;
    if base != model::BASE_MISSING && !crate::native_mcp::model::is_sha256_hex(base) {
        bail!("--base 必须是 64 位小写 sha256 或 missing");
    }
    Ok(base.to_string())
}

/// 严格参数：只接受 `--only <id> --base <digest>`。
fn check_args(req: &Request, command: &str) -> Result<()> {
    if req.flag("--dry-run") {
        bail!("{command} 已退役 --dry-run，拒绝该参数");
    }
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--only" | "--base" => {
                iter.next();
            }
            other => bail!("{command} 不支持参数 '{other}'"),
        }
    }
    Ok(())
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

/// 实例目标文件：解析符号链接到最终普通文件（与 hooks、接入配置一致），返回
/// （展示用文件名, 实际读写路径, 现有字节）。
fn read_target(def: &ProfileDef, dir: &Path) -> Result<(String, PathBuf, Option<Vec<u8>>)> {
    let file = def.kind.target_file().to_string();
    let path = dir.join(&file);
    let real = crate::native_agent_settings::resolve_write_target(&path)
        .map_err(|err| anyhow!("[{}] {err:#}", def.id))?;
    let bytes = match std::fs::symlink_metadata(&real) {
        Ok(meta) if meta.is_file() => Some(std::fs::read(&real)?),
        Ok(_) => bail!("[{}] {} 不是普通文件，拒绝写入", def.id, file),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err.into()),
    };
    Ok((file, real, bytes))
}

/// 上次写入记下的路径（没有记录时为空）。
fn recorded_paths(ctx: &Context, def: &ProfileDef, file: &str) -> Result<Vec<String>> {
    Ok(match store::open_readonly(ctx)? {
        Some(conn) => load_bindings_from(&conn)?
            .get(&(def.id.clone(), file.to_string()))
            .map(|b| b.owned.clone())
            .unwrap_or_default(),
        None => Vec::new(),
    })
}

fn stale(def: &ProfileDef, file: &str) -> anyhow::Error {
    anyhow!(
        "[{}] {} 已在外部修改（与打开时读到的内容不同），没有写入；请刷新后重新操作",
        def.id,
        file
    )
}

/// 一次写入的计划：锁前与锁内各算一次，锁内结果为准。
struct Plan {
    def: ProfileDef,
    dir: PathBuf,
    file: String,
    real: PathBuf,
    previous: Option<Vec<u8>>,
    content: Vec<u8>,
}

fn plan(ctx: &Context, req: &Request) -> Result<Plan> {
    check_args(req, "profiles:apply")?;
    let id = parse_only(req)?;
    let base = parse_base(req)?;
    let def = find_def(ctx, &id)?;
    if !def.enabled {
        bail!("[{}] 实例已停用，拒绝写入", id);
    }
    let dir = model::validate_config_dir(&def, &ctx.home_dir, &ctx.data_dir)?;
    let (file, real, previous) = read_target(&def, &dir)?;
    if model::live_base(def.kind, previous.as_deref())? != base {
        return Err(stale(&def, &file));
    }
    let recorded = recorded_paths(ctx, &def, &file)?;
    // 渲染：变量缺失、现有文件损坏在取锁前就暴露
    let rendered = model::render_all(
        &def,
        &|_| previous.clone(),
        &crate::secret_values::lookup,
        &recorded,
    )?;
    let content = rendered
        .into_iter()
        .next()
        .map(|target| target.content)
        .ok_or_else(|| anyhow!("[{}] 没有要写的文件", id))?;
    Ok(Plan {
        def,
        dir,
        file,
        real,
        previous,
        content,
    })
}

/// 第三方实例的配置目录里不能有官方账号登录：两种凭据在同一个目录里，助手取哪个就只能
/// 靠它的优先级规则，一旦第三方凭据缺失就会退回官方登录。用官方命令确认没有才写；
/// 无法确认也拒绝。官方账号实例（不带第三方凭据与地址）不做这项检查。
fn ensure_no_official_login(ctx: &Context, def: &ProfileDef, dir: &Path) -> Result<()> {
    if !model::is_third_party(def) {
        return Ok(());
    }
    let kind = match def.kind {
        model::ProfileKind::Claude => crate::native_auth::AuthKind::Claude,
        model::ProfileKind::Codex => crate::native_auth::AuthKind::Codex,
    };
    let bin = super::status::base_bin(ctx, def);
    match crate::native_auth::official_login(&ctx.home_dir, kind, &bin, dir) {
        Ok(false) => Ok(()),
        Ok(true) => bail!(
            "[{}] {} 里已有官方账号登录，第三方凭据不能写进同一个目录；请给这套第三方接入换一个新的配置目录",
            def.id,
            def.config_dir_raw
        ),
        Err(reason) => bail!(
            "[{}] 无法确认 {} 里没有官方账号登录（{reason}），没有写入",
            def.id,
            def.config_dir_raw
        ),
    }
}

/// 锁前预检：参数与状态校验，全程零写。
pub(crate) fn preflight_apply(ctx: &Context, req: &Request) -> Result<()> {
    let cmd = req.command.replace('.', ":");
    if cmd == "profiles:adopt" {
        plan_adopt(ctx, req)?;
    } else {
        plan(ctx, req)?;
    }
    Ok(())
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

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) fn apply(ctx: &Context, req: &Request) -> Result<Value> {
    let cmd = req.command.replace('.', ":");
    if cmd == "profiles:adopt" {
        return adopt(ctx, req);
    }
    // 锁内重新读取并比对基准，不沿用锁前结果
    let plan = plan(ctx, req)?;
    let def = &plan.def;
    ensure_no_official_login(ctx, def, &plan.dir)?;

    ensure_dir(&plan.dir)?;
    let existed = plan.previous.is_some();
    // 写前再比一次字节：锁内读取之后若又被改过，拒绝替换
    let current = std::fs::read(&plan.real).ok();
    if current != plan.previous {
        return Err(stale(def, &plan.file));
    }
    atomic_write(&plan.real, &plan.content)?;
    let rollback = || match &plan.previous {
        Some(bytes) => {
            let _ = std::fs::write(&plan.real, bytes);
        }
        None => {
            let _ = std::fs::remove_file(&plan.real);
        }
    };

    let result = (|| -> Result<()> {
        let mut conn = store::open_write(ctx)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        write_binding(
            &tx,
            &Binding {
                profile_id: def.id.clone(),
                file: plan.file.clone(),
                applied_digest: {
                    let base = model::live_base(def.kind, Some(&plan.content))?;
                    if base == model::BASE_MISSING {
                        crate::native_mcp::model::sha256_hex_bytes(&plan.content)
                    } else {
                        base
                    }
                },
                updated_at: now(),
                owned: model::owned_paths(def),
            },
        )?;
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "profiles:apply",
            Some(&def.id),
            Some(&json!({
                "action": "apply",
                "kind": def.kind.as_str(),
                "created": !existed,
                "files": [plan.file.clone()],
            })),
        )?;
        tx.commit()?;
        Ok(())
    })();

    if let Err(err) = result {
        rollback();
        return Err(err);
    }

    Ok(json!({
        "domain": "profiles",
        "steps": [{"name": format!("profiles:apply {}", def.id), "ok": true, "exit_code": 0}],
        "skipped": [],
        "base": model::live_base(def.kind, Some(&plan.content))?,
    }))
}

/// 采纳计划：新 profile 段与提示。
struct AdoptPlan {
    def: ProfileDef,
    profile: serde_json::Map<String, Value>,
    notes: Vec<String>,
}

fn plan_adopt(ctx: &Context, req: &Request) -> Result<AdoptPlan> {
    check_args(req, "profiles:adopt")?;
    let id = parse_only(req)?;
    let base = parse_base(req)?;
    let def = find_def(ctx, &id)?;
    let dir = model::validate_config_dir(&def, &ctx.home_dir, &ctx.data_dir)?;
    let (file, _real, bytes) = read_target(&def, &dir)?;
    if model::live_base(def.kind, bytes.as_deref())? != base {
        return Err(stale(&def, &file));
    }
    let policy = store::effective_policy(ctx)?;
    let raw_profile = policy
        .get("agents")
        .and_then(|v| v.get("definitions"))
        .and_then(|v| v.get(&id))
        .and_then(|v| v.get("profile"))
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| anyhow!("未知实例: {}", id))?;
    let (profile, notes) = model::adopt_profile(
        &def,
        &raw_profile,
        bytes.as_deref(),
        &crate::secret_values::lookup,
    )?;
    // 采纳结果必须仍是合法定义（凭据仍只能是引用等），否则整体拒绝
    let mut candidate = policy
        .get("agents")
        .and_then(|v| v.get("definitions"))
        .and_then(|v| v.get(&id))
        .cloned()
        .unwrap_or_else(|| json!({}));
    candidate["profile"] = Value::Object(profile.clone());
    model::parse_definition(&id, &candidate, true)
        .map_err(|err| anyhow!("[{id}] 实际配置无法作为实例定义保存：{err:#}"))?;
    Ok(AdoptPlan {
        def,
        profile,
        notes,
    })
}

/// 以实际配置更新实例定义：只写库。
fn adopt(ctx: &Context, req: &Request) -> Result<Value> {
    let plan = plan_adopt(ctx, req)?;
    let id = plan.def.id.clone();
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut policy = store::policy_from_connection(&tx)?;
    let definition = policy
        .get_mut("agents")
        .and_then(|v| v.get_mut("definitions"))
        .and_then(|v| v.get_mut(&id))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("未知实例: {}", id))?;
    definition.insert("profile".into(), Value::Object(plan.profile));
    store::write_policy(&tx, &policy)?;
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "profiles:adopt",
        Some(&id),
        Some(
            &json!({"action": "adopt", "kind": plan.def.kind.as_str(), "notes": plan.notes.len()}),
        ),
    )?;
    tx.commit()?;
    Ok(json!({
        "domain": "profiles",
        "steps": [{"name": format!("profiles:adopt {id}"), "ok": true, "exit_code": 0}],
        "skipped": [],
        "notes": plan.notes,
    }))
}
