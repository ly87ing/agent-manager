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
use super::status::definition_base;
use crate::native::{Context, Request};
use crate::store;

const MAX_PREVIEW_BYTES: usize = 4 * 1024 * 1024;

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

/// 更新定义并写入文件只接受候选受管字段和两个独立的写入基准。
fn check_update_args(req: &Request) -> Result<()> {
    if req.flag("--dry-run") {
        bail!("profiles:apply-definition 不支持 --dry-run");
    }
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--only" | "--base" | "--definition-base" | "--candidate" => {
                if iter.next().is_none() {
                    bail!("profiles:apply-definition 的 {arg} 缺少取值");
                }
            }
            other => bail!("profiles:apply-definition 不支持参数 '{other}'"),
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

fn plan_for_def(
    ctx: &Context,
    req: &Request,
    def: ProfileDef,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> Result<Plan> {
    let base = parse_base(req)?;
    if !def.enabled {
        bail!("[{}] 实例已停用，拒绝写入", def.id);
    }
    let dir = model::validate_config_dir(&def, &ctx.home_dir, &ctx.data_dir)?;
    let (file, real, previous) = read_target(&def, &dir)?;
    if model::live_base(def.kind, previous.as_deref())? != base {
        return Err(stale(&def, &file));
    }
    let recorded = recorded_paths(ctx, &def, &file)?;
    // 渲染：变量缺失、现有文件损坏在取锁前就暴露
    let rendered = model::render_all(&def, &|_| previous.clone(), resolve, &recorded)?;
    let content = rendered
        .into_iter()
        .next()
        .map(|target| target.content)
        .ok_or_else(|| anyhow!("[{}] 没有要写的文件", def.id))?;
    Ok(Plan {
        def,
        dir,
        file,
        real,
        previous,
        content,
    })
}

fn plan(ctx: &Context, req: &Request) -> Result<Plan> {
    check_args(req, "profiles:apply")?;
    let id = parse_only(req)?;
    plan_for_def(ctx, req, find_def(ctx, &id)?, &crate::secret_values::lookup)
}

/// 按表单草稿只读渲染目标文件。变量取值保留引用形式，不读取密钥来源；保存时仍按实际文件重新计算。
pub(crate) fn preview(ctx: &Context, req: &Request) -> Result<Value> {
    if req.args.iter().any(|arg| arg == "--candidate") {
        let (plan, _, _) =
            plan_update_with_resolver(ctx, req, &|name| Some(format!("${{{name}}}")))?;
        if plan
            .previous
            .as_ref()
            .is_some_and(|bytes| bytes.len() > MAX_PREVIEW_BYTES)
            || plan.content.len() > MAX_PREVIEW_BYTES
        {
            bail!("配置文件超过安全读取上限");
        }
        return Ok(json!({
            "element": format!("profiles:{}", plan.def.id),
            "path": format!("{}/{}", plan.def.config_dir_raw, plan.file),
            "format": if plan.def.kind.as_str() == "codex" { "toml" } else { "json" },
            "exists": plan.previous.is_some(),
            "content": String::from_utf8(plan.content).map_err(|_| anyhow!("预览结果不是 UTF-8 文本"))?,
        }));
    }
    if !matches!(req.args.len(), 4 | 6)
        || req.args[0] != "--only"
        || req.args[2] != "--definition"
        || (req.args.len() == 6 && req.args[4] != "--items")
    {
        bail!("profiles:preview 只接受 --only profiles:<实例名> --definition <JSON> [--items <逗号分隔的共用设置>]");
    }
    let id = parse_only(req)?;
    let raw: Value =
        serde_json::from_str(&req.args[3]).map_err(|_| anyhow!("--definition 不是合法 JSON"))?;
    let def = model::parse_definition(&id, &raw, true)?;
    let dir = model::validate_config_dir(&def, &ctx.home_dir, &ctx.data_dir)?;
    let (file, _, previous) = read_target(&def, &dir)?;
    if previous
        .as_ref()
        .is_some_and(|bytes| bytes.len() > MAX_PREVIEW_BYTES)
    {
        bail!("配置文件超过安全读取上限");
    }
    // 官方 Codex 新环境只建立定义和启动命令；主文件要等勾选的共用设置来写。
    let content = if def.kind == model::ProfileKind::Codex && !model::is_third_party(&def) {
        previous.clone().unwrap_or_default()
    } else {
        let recorded = recorded_paths(ctx, &def, &file)?;
        model::render_all(
            &def,
            &|_| previous.clone(),
            &|name| Some(format!("${{{name}}}")),
            &recorded,
        )?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("[{}] 没有可预览的文件", def.id))?
        .content
    };
    let items = if req.args.len() == 6 {
        super::seed::parse_items(&req.args[5])?
    } else {
        Vec::new()
    };
    let seed_preview = super::preview_seed::preview(ctx, req, &def, &dir, content, &items);
    let will_write = def.kind == model::ProfileKind::Claude
        || model::is_third_party(&def)
        || seed_preview.results.iter().any(|item| {
            item["state"] == "copied"
                && matches!(
                    item["item"].as_str(),
                    Some("permissions" | "statusline" | "mcp")
                )
        });
    let content = seed_preview.main;
    if content.len() > MAX_PREVIEW_BYTES {
        bail!("配置文件超过安全读取上限");
    }
    Ok(json!({
        "element": format!("profiles:{id}"),
        "path": format!("{}/{}", def.config_dir_raw, file),
        "format": if def.kind.as_str() == "codex" { "toml" } else { "json" },
        "exists": previous.is_some(),
        "will_write": will_write,
        "content": String::from_utf8(content).map_err(|_| anyhow!("预览结果不是 UTF-8 文本"))?,
        "seed_results": seed_preview.results,
        "files": seed_preview.files,
    }))
}

/// 从现有完整定义生成候选；只替换实例负责的字段，未知元数据原样保留。
fn plan_update(ctx: &Context, req: &Request) -> Result<(Plan, Value, String)> {
    plan_update_with_resolver(ctx, req, &crate::secret_values::lookup)
}

fn plan_update_with_resolver(
    ctx: &Context,
    req: &Request,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> Result<(Plan, Value, String)> {
    check_update_args(req)?;
    let id = parse_only(req)?;
    let expected = req
        .value("--definition-base")
        .ok_or_else(|| anyhow!("缺少 --definition-base"))?;
    if !crate::native_mcp::model::is_sha256_hex(expected) {
        bail!("--definition-base 必须是 64 位小写 sha256");
    }
    let policy = store::effective_policy(ctx)?;
    let original = policy
        .get("agents")
        .and_then(|v| v.get("definitions"))
        .and_then(|v| v.get(&id))
        .ok_or_else(|| anyhow!("未知实例: {}", id))?;
    if definition_base(original)? != expected {
        bail!("[{}] 实例定义已在外部修改，没有写入；请刷新后重试", id);
    }
    let current = find_def(ctx, &id)?;
    let raw_candidate = req
        .value("--candidate")
        .ok_or_else(|| anyhow!("缺少 --candidate"))?;
    let candidate: Value =
        serde_json::from_str(raw_candidate).map_err(|_| anyhow!("--candidate 不是合法 JSON"))?;
    let fields = candidate
        .as_object()
        .ok_or_else(|| anyhow!("--candidate 必须是对象"))?;
    let owned: &[&str] = match current.kind {
        model::ProfileKind::Claude => &["env", "model"],
        model::ProfileKind::Codex => &["base_url", "env_key", "wire_api", "model"],
    };
    if fields
        .keys()
        .any(|key| key != "source_connection" && !owned.contains(&key.as_str()))
    {
        bail!("--candidate 只能更新该助手实例的接入字段");
    }
    let source = fields
        .get("source_connection")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("--candidate 缺少来源 Provider 名称"))?;
    if source.is_empty()
        || source.len() > 64
        || !source
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("来源 Provider 名称格式不正确");
    }
    let old_source = original
        .get("profile")
        .and_then(|v| v.get("source_connection"))
        .and_then(Value::as_str);
    if old_source.is_some_and(|old| old != source) {
        bail!("[{}] 来源 Provider 与现有定义不一致，没有写入", id);
    }
    let mut next = original.clone();
    let profile = next
        .get_mut("profile")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("[{}] 缺少 profile 段", id))?;
    for key in owned {
        profile.remove(*key);
    }
    profile.extend(fields.clone());
    let def = model::parse_definition(&id, &next, current.enabled)?;
    let has_third_party_credential = match def.kind {
        model::ProfileKind::Claude => {
            def.env.contains_key("ANTHROPIC_BASE_URL")
                && (def.env.contains_key("ANTHROPIC_AUTH_TOKEN")
                    || def.env.contains_key("ANTHROPIC_API_KEY"))
        }
        model::ProfileKind::Codex => def.base_url.is_some() && def.env_key.is_some(),
    };
    if !has_third_party_credential {
        bail!(
            "[{}] Provider 更新必须保留独立环境的第三方地址与凭据引用",
            id
        );
    }
    let plan = plan_for_def(ctx, req, def, resolve)?;
    Ok((plan, next, expected.to_string()))
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
    } else if cmd == "profiles:apply-definition" {
        plan_update(ctx, req)?;
    } else {
        plan(ctx, req)?;
    }
    Ok(())
}

/// 原子换入：同目录临时文件 + rename，权限 0600。文件里有凭据：临时文件从创建起就是 0600，
/// 且只新建不复用——上次中断留下的同名文件或链接先删掉，不会顺着链接把内容写到别处。
fn atomic_write(target: &Path, content: &[u8]) -> Result<()> {
    use std::io::Write;
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
    match std::fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(content)?;
    file.sync_all()?;
    drop(file);
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
    let (plan, update) = if cmd == "profiles:apply-definition" {
        let (plan, candidate, expected) = plan_update(ctx, req)?;
        (plan, Some((candidate, expected)))
    } else {
        (plan(ctx, req)?, None)
    };
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
        if let Some((candidate, expected)) = &update {
            let mut policy = store::policy_from_connection(&tx)?;
            let definitions = policy
                .get_mut("agents")
                .and_then(|v| v.get_mut("definitions"))
                .and_then(Value::as_object_mut)
                .ok_or_else(|| anyhow!("[{}] 实例定义已移除，没有写入", def.id))?;
            let current = definitions
                .get(&def.id)
                .ok_or_else(|| anyhow!("[{}] 实例定义已移除，没有写入", def.id))?;
            if definition_base(current)? != *expected {
                bail!("[{}] 实例定义已在外部修改，没有写入", def.id);
            }
            definitions.insert(def.id.clone(), candidate.clone());
            store::write_policy(&tx, &policy)?;
        }
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
            if update.is_some() {
                "profiles:apply-definition"
            } else {
                "profiles:apply"
            },
            Some(&def.id),
            Some(&json!({
                "action": if update.is_some() { "apply-definition" } else { "apply" },
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
        "steps": [{"name": format!("{} {}", req.command.replace('.', ":"), def.id), "ok": true, "exit_code": 0}],
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 临时文件路径上残留的链接不会被顺着写出去() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("am-profile-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("instance");
        std::fs::create_dir_all(&dir).unwrap();
        let outside = root.join("outside.txt");
        std::fs::write(&outside, "keep").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join(".config.toml.am-tmp")).unwrap();

        atomic_write(&dir.join("config.toml"), b"token = \"x\"\n").unwrap();

        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep");
        assert_eq!(
            std::fs::read(dir.join("config.toml")).unwrap(),
            b"token = \"x\"\n"
        );
        let mode = std::fs::metadata(dir.join("config.toml"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(!dir.join(".config.toml.am-tmp").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
