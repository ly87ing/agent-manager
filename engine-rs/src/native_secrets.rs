//! Rust 原生 secrets 域：外部密钥文件状态、取值写入与旧数据库凭据迁移。
//!
//! 唯一元素 secrets:mcp-env：事实源为各助手 MCP 配置里的实际条目与本工具里的
//! 停用快照，停用的条目同样计入（重新启用时要用到）；
//! 对每个变量引用（stdio env 与远程 headers 同级）输出确定性行
//! {server, env_key, placeholder, resolved, kind}，kind 为 "env" 或 "header"；
//! 字面量不是变量，不列出。只读状态可观察进程环境，但账号受控操作必须从显式选择的
//! 外部 `.env` 文件读取；`stored` 说明取值是否来自外部文件。
//! 任何状态、操作记录与配置备份都不保存、不序列化、不输出变量值本身。
//!
//! `secrets:set --var NAME --value V` 只写用户明确选择的外部文件（空值即删除）；
//! MCP 不自动收养或复制明文凭据；变量归属和账号代际只记录名称与身份绑定。
//! `secrets:migrate-legacy` 需显式触发，先迁移旧表取值再删表。错误消息一律不回显取值。

use std::collections::BTreeSet;

use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::native::{Context, Request};
use crate::native_mcp::entry::{self, all_referenced_vars, referenced_vars};
use crate::native_mcp::targets::Format;
use crate::store;

type VarRef = (String, String, String, String);

fn add_entry_refs(refs: &mut BTreeSet<VarRef>, server: &str, format: &Format, value: &Value) {
    if let Ok(view) = entry::parse(format, value) {
        for (key, kind, var) in referenced_vars(format, &view) {
            refs.insert((server.to_string(), key, kind.to_string(), var));
        }
    }
}

/// MCP 实际条目与已保存项里引用的变量：(server, env_key, kind, 变量名)，去重。
fn mcp_references(ctx: &Context) -> Result<BTreeSet<VarRef>> {
    let inv = crate::native_mcp::inventory::load(ctx)?;
    let mut refs = BTreeSet::new();
    for scan in &inv.scans {
        for item in &scan.entries {
            add_entry_refs(&mut refs, &item.name, &scan.target.format, &item.raw);
        }
    }
    // 停用快照写回时要用到这些变量；未分配的已保存定义要等加到某个助手时才用得上，不列。
    for row in inv.saved.iter().filter(|row| !row.agent.is_empty()) {
        if let Some(scan) = inv.scan(&row.agent) {
            add_entry_refs(&mut refs, &row.server, &scan.target.format, &row.entry);
        }
    }
    Ok(refs)
}

/// 接入配置与独立环境引用的凭据变量：变量名 → 用在哪些地方。只看变量名，不读取值。
///
/// 独立环境写配置时把取值写进了自己的文件，改了取值要到那个环境重新写入；接入配置只在检查或
/// 建成独立环境时解析。界面据此说明「改了之后要做什么」。
fn provider_key_references(ctx: &Context) -> std::collections::BTreeMap<String, Vec<Value>> {
    let mut refs: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
    if let Ok(defs) = crate::native_agents::load_definitions(ctx) {
        for def in defs
            .iter()
            .filter(|def| def.spec_ref().get("profile").is_none())
        {
            let Ok(settings) = crate::native_agent_settings::parse(def.spec_ref()) else {
                continue;
            };
            for (name, profile) in &settings.profiles {
                if let Some(var) = &profile.token_var {
                    refs.entry(var.clone()).or_default().push(json!({
                        "kind": "connection",
                        "agent": def.id,
                        "name": name,
                        "label": profile.label.clone().unwrap_or_else(|| name.clone()),
                    }));
                }
            }
        }
    }
    if let Ok(policy) = store::effective_policy(ctx) {
        for def in crate::native_profiles::model::load_definitions(&policy)
            .into_iter()
            .flatten()
        {
            let mut vars: Vec<String> = def
                .env
                .values()
                .filter_map(Value::as_str)
                .filter_map(|text| text.strip_prefix("${")?.strip_suffix('}'))
                .map(str::to_string)
                .collect();
            vars.extend(def.env_key.clone());
            vars.sort();
            vars.dedup();
            for var in vars {
                refs.entry(var).or_default().push(json!({
                    "kind": "profile",
                    "id": def.id,
                    "label": def.display_name,
                }));
            }
        }
    }
    refs
}

/// Credential references are account-owned, including disabled and malformed registrations.
/// Saved connection templates do not own an identity until assigned to an account.
pub(crate) fn account_secret_owners(
    ctx: &Context,
) -> Result<std::collections::BTreeMap<String, BTreeSet<String>>> {
    let policy = store::effective_policy(ctx)?;
    account_secret_owners_in_policy(ctx, &policy)
}

fn implicit_native_mcp_refs(
    ctx: &Context,
    policy: &Value,
    enabled_only: bool,
    only: Option<&str>,
) -> Result<std::collections::BTreeMap<String, BTreeSet<String>>> {
    use crate::native_profiles::{model, paths};
    let mut out = std::collections::BTreeMap::new();
    let Some(defs) = policy
        .pointer("/agents/definitions")
        .and_then(Value::as_object)
    else {
        return Ok(out);
    };
    for (id, raw) in defs {
        if only.is_some_and(|only| only != id) {
            continue;
        }
        if raw.get("_delete") == Some(&Value::Bool(true)) {
            continue;
        }
        let (path, format) = if raw.get("profile").is_some() {
            let Ok(def) = model::parse_definition(id, raw, true) else {
                continue;
            };
            let dir = paths::config_dir_in_policy(ctx, &def, policy)?;
            let filename = if def.kind == model::ProfileKind::Claude {
                ".claude.json"
            } else {
                "config.toml"
            };
            (
                paths::target_in_policy(ctx, &def, &dir.join(filename), policy)?,
                if def.kind == model::ProfileKind::Claude {
                    Format::ClaudeJson
                } else {
                    Format::CodexToml
                },
            )
        } else if id == "claude-code" {
            (ctx.home_dir.join(".claude.json"), Format::ClaudeJson)
        } else if id == "codex" {
            (ctx.home_dir.join(".codex/config.toml"), Format::CodexToml)
        } else {
            continue;
        };
        let path = paths::validate_account_resource_path(ctx, policy, id, &path)?;
        let bytes = model::read_account_file(&path)?;
        let (entries, _) =
            crate::native_mcp::inventory::parse_container(&format, &path, bytes.as_deref())
                .map_err(|_| anyhow::anyhow!("账号原生 MCP 配置不可解析，无法确认密钥引用归属"))?;
        let mut names = BTreeSet::new();
        for (_, entry) in entries {
            let view = crate::native_mcp::entry::parse(&format, &entry)
                .map_err(|_| anyhow::anyhow!("账号原生 MCP 条目不可解析，无法确认密钥引用归属"))?;
            if enabled_only && !view.enabled {
                continue;
            }
            names.extend(all_referenced_vars(&format, &view)?);
        }
        out.insert(id.clone(), names);
    }
    Ok(out)
}

fn load_variable_claims(conn: &rusqlite::Connection) -> Result<Vec<(String, String, String)>> {
    let mut stmt = match conn
        .prepare("SELECT name, account, binding FROM secret_variable_owners ORDER BY name")
    {
        Ok(stmt) => stmt,
        Err(err) if err.to_string().contains("no such table") => return Ok(Vec::new()),
        Err(_) => bail!("持久密钥归属记录不可读取，拒绝操作"),
    };
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| anyhow::anyhow!("持久密钥归属记录已损坏，拒绝操作"))
}

fn credential_claim_binding(
    ctx: &Context,
    policy: &Value,
    account: &str,
) -> Result<Option<String>> {
    use crate::native_profiles::{model, paths};
    let registered_generation = |raw: &Value| -> Result<String> {
        let generation = raw
            .get("account_generation")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "账号缺少安全登记代际，请显式重新保存定义后操作密钥；没有迁移或覆盖凭据"
                )
            })?;
        if generation.len() != 36
            || !generation.chars().enumerate().all(|(i, c)| {
                if [8, 13, 18, 23].contains(&i) {
                    c == '-'
                } else {
                    c.is_ascii_hexdigit()
                }
            })
        {
            bail!("账号登记代际无效，拒绝更改密钥归属");
        }
        Ok(generation.to_string())
    };
    let Some(raw) = policy
        .pointer("/agents/definitions")
        .and_then(|defs| defs.get(account))
    else {
        return Ok(None);
    };
    if raw.get("_delete") == Some(&Value::Bool(true)) {
        return Ok(None);
    }
    let (generation, location, kind, mode) = if raw.get("profile").is_some() {
        let def = model::parse_definition(account, raw, true)?;
        let generation = registered_generation(raw)?;
        (
            generation,
            paths::config_dir_in_policy(ctx, &def, policy)?,
            def.kind.as_str().to_string(),
            format!("{:?}", model::account_auth_mode(&def)),
        )
    } else if account == "claude-code" || account == "codex" {
        (
            "reserved-primary-v1".into(),
            ctx.home_dir.join(if account == "claude-code" {
                ".claude"
            } else {
                ".codex"
            }),
            if account == "claude-code" {
                "claude"
            } else {
                "codex"
            }
            .into(),
            "Official".into(),
        )
    } else {
        let target = crate::native_mcp::targets::agent_targets(policy, &ctx.home_dir)
            .into_iter()
            .find(|target| target.agent == account)
            .ok_or_else(|| anyhow::anyhow!("密钥所属账号没有可验证的原生目标，拒绝操作"))?;
        (
            if account == "opencode" {
                "reserved-primary-mcp-v1".into()
            } else {
                registered_generation(raw)?
            },
            paths::validate_account_resource_path(ctx, policy, account, &target.path)?,
            target.format.label().to_string(),
            "McpOnly".into(),
        )
    };
    let identity = json!({"version":1,"account":account,"generation":generation,
        "home":crate::native_mcp::targets::resolved_path(&ctx.home_dir),
        "location":crate::native_mcp::targets::resolved_path(&location),"kind":kind,"auth_mode":mode});
    Ok(Some(crate::native_mcp::model::sha256_hex(
        &serde_json::to_string(&identity)?,
    )))
}

/// Reserve a previous registration's names before changing/removing its definition.
/// Legacy first-save can use the new UUID only when kind, mode and directory are unchanged.
pub(crate) fn reserve_account_references(
    ctx: &Context,
    conn: &rusqlite::Connection,
    account: &str,
    before: &Value,
    after: Option<&Value>,
) -> Result<()> {
    let raw = before
        .pointer("/agents/definitions")
        .and_then(|defs| defs.get(account));
    if raw.is_none_or(|raw| raw.get("_delete") == Some(&Value::Bool(true))) {
        return Ok(());
    }
    let raw = raw.unwrap();
    let mut names = BTreeSet::new();
    if let Some(profile) = raw.get("profile") {
        collect_placeholders(profile, &mut names);
        if let Some(name) = profile.get("env_key").and_then(Value::as_str) {
            names.insert(name.to_string());
        }
    }
    // A profile-field deletion leaves its MCP path and old generation in policy.
    // Read that former identity as an MCP-only reservation, with the same generic
    // protected-path checks; it is never a launchable primary-Claude registration.
    let mut reservation_policy = before.clone();
    if raw.get("profile").is_none()
        && raw.get("account_generation").is_some()
        && account != "claude-code"
        && account != "codex"
    {
        if let Some(def) = reservation_policy
            .pointer_mut("/agents/definitions")
            .and_then(|defs| defs.get_mut(account))
            .and_then(Value::as_object_mut)
        {
            def.remove("bin");
        }
    }
    match account_reference_names(ctx, account, &reservation_policy) {
        Ok(actual) => names.extend(actual),
        Err(_)
            if raw.get("profile").is_some()
                && crate::native_profiles::model::parse_definition(account, raw, true).is_err() => {
        }
        Err(err) => {
            return Err(err.context("旧账号配置不可可靠读取，无法保留原密钥归属；拒绝更改定义"))
        }
    }
    if names.is_empty() {
        return Ok(());
    }
    let old_binding = credential_claim_binding(ctx, before, account)
        .ok()
        .flatten();
    let compatible_after = after.and_then(|policy| {
        let next = policy
            .pointer("/agents/definitions")
            .and_then(|defs| defs.get(account))?;
        if raw.get("account_generation").is_none()
            && raw.get("profile").is_none()
            && next.get("profile").is_none()
            && raw.get("mcp_config").is_some()
            && raw.get("mcp_config") == next.get("mcp_config")
            && next.get("_delete") != Some(&Value::Bool(true))
        {
            return credential_claim_binding(ctx, policy, account)
                .ok()
                .flatten();
        }
        let old = crate::native_profiles::model::parse_definition(account, raw, true).ok()?;
        let new = crate::native_profiles::model::parse_definition(account, next, true).ok()?;
        if raw.get("account_generation").is_some()
            || old.kind != new.kind
            || crate::native_profiles::model::account_auth_mode(&old)
                != crate::native_profiles::model::account_auth_mode(&new)
            || old.config_dir_raw != new.config_dir_raw
            || next.get("_delete") == Some(&Value::Bool(true))
        {
            return None;
        }
        credential_claim_binding(ctx, policy, account)
            .ok()
            .flatten()
    });
    let binding = old_binding.or(compatible_after).unwrap_or_else(|| {
        crate::native_mcp::model::sha256_hex(&json!({"version":1,"retired":true,"account":account,"home":ctx.home_dir,"registration":raw}).to_string())
    });
    conn.execute_batch("CREATE TABLE IF NOT EXISTS secret_variable_owners (name TEXT PRIMARY KEY NOT NULL, account TEXT NOT NULL, binding TEXT NOT NULL)")?;
    // Existing claims retain their original provenance; never move or overwrite a claim.
    for name in names {
        if !crate::secret_values::valid_var_name(&name) {
            bail!("旧账号引用变量名不合法，拒绝丢弃原归属");
        }
        conn.execute("INSERT INTO secret_variable_owners(name,account,binding) VALUES (?1,?2,?3) ON CONFLICT(name) DO NOTHING", rusqlite::params![name.to_ascii_uppercase(),account,binding])?;
    }
    Ok(())
}

/// Only reference names and account registration binding are persisted, never values.
/// Callers hold account locks and include this call in the file/definition transaction.
pub(crate) fn claim_variable_ownership(
    ctx: &Context,
    conn: &rusqlite::Connection,
    account: &str,
    names: &BTreeSet<String>,
) -> Result<()> {
    if names.is_empty() {
        return Ok(());
    }
    if names
        .iter()
        .any(|name| !crate::secret_values::valid_var_name(name))
    {
        bail!("密钥变量名称不合法，拒绝登记归属");
    }
    let policy = store::policy_from_connection(conn)?;
    let binding = credential_claim_binding(ctx, &policy, account)?
        .ok_or_else(|| anyhow::anyhow!("密钥所属账号已删除或未登记，拒绝操作"))?;
    let owners = account_secret_owners_in_policy(ctx, &policy)?;
    for name in names {
        if owners
            .get(&name.to_ascii_uppercase())
            .is_some_and(|ids| ids.iter().any(|id| id != account))
        {
            bail!("密钥变量已属于另一账号或已退役的账号代际，拒绝复用；请显式配置新的独占变量");
        }
    }
    for (name, owner, saved_binding) in load_variable_claims(conn)? {
        if names
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(&name))
            && (owner != account || saved_binding != binding)
        {
            bail!("密钥变量已属于另一账号或已退役的账号代际，拒绝复用；请显式配置新的独占变量");
        }
    }
    conn.execute_batch("CREATE TABLE IF NOT EXISTS secret_variable_owners (name TEXT PRIMARY KEY NOT NULL, account TEXT NOT NULL, binding TEXT NOT NULL)")?;
    for name in names {
        conn.execute("INSERT INTO secret_variable_owners(name,account,binding) VALUES (?1,?2,?3) ON CONFLICT(name) DO NOTHING", rusqlite::params![name.to_ascii_uppercase(),account,binding])?;
    }
    Ok(())
}

pub(crate) fn account_secret_owners_in_policy(
    ctx: &Context,
    policy: &Value,
) -> Result<std::collections::BTreeMap<String, BTreeSet<String>>> {
    let mut owners: std::collections::BTreeMap<String, BTreeSet<String>> = Default::default();
    if let Some(defs) = policy
        .get("agents")
        .and_then(|a| a.get("definitions"))
        .and_then(Value::as_object)
    {
        for (id, raw) in defs {
            if raw.get("_delete") == Some(&Value::Bool(true)) {
                continue;
            }
            let Some(profile) = raw.get("profile") else {
                continue;
            };
            let mut names = BTreeSet::new();
            collect_placeholders(profile, &mut names);
            if let Some(name) = profile.get("env_key").and_then(Value::as_str) {
                names.insert(name.to_string());
            }
            for name in names {
                owners
                    .entry(name.to_ascii_uppercase())
                    .or_default()
                    .insert(id.clone());
            }
        }
    }
    if let Some(conn) = store::open_readonly(ctx)? {
        for (name, account, saved_binding) in load_variable_claims(&conn)? {
            if !crate::secret_values::valid_var_name(&name) {
                bail!("持久密钥归属记录已损坏，拒绝操作");
            }
            let owner = if credential_claim_binding(ctx, policy, &account)
                .ok()
                .flatten()
                .as_deref()
                == Some(saved_binding.as_str())
            {
                account
            } else {
                format!("retired:{account}")
            };
            owners
                .entry(name.to_ascii_uppercase())
                .or_default()
                .insert(owner);
        }
    }
    for (id, names) in implicit_native_mcp_refs(ctx, policy, false, None)? {
        for name in names {
            owners
                .entry(name.to_ascii_uppercase())
                .or_default()
                .insert(id.clone());
        }
    }
    let scans = crate::native_mcp::inventory::scan_targets_checked(ctx, policy);
    let rows = crate::native_mcp::saved::load_readonly(ctx)?;
    let bindings = crate::native_mcp::legacy::load_bindings_readonly(ctx)?;
    let derived = crate::native_mcp::legacy::derive(policy, &bindings, &scans);
    let inv = crate::native_mcp::inventory::Inventory {
        scans,
        saved: crate::native_mcp::inventory::merge_saved(rows, derived),
    };
    for scan in &inv.scans {
        if scan.error.is_some() {
            anyhow::bail!("账号 MCP 配置不可读取或解析，无法确认密钥引用归属，拒绝操作");
        }
        for item in &scan.entries {
            let view = item.view.as_ref().map_err(|_| {
                anyhow::anyhow!("账号 MCP 条目不可解析，无法确认密钥引用归属，拒绝操作")
            })?;
            let mut names = BTreeSet::new();
            collect_placeholders(&item.raw, &mut names);
            names.extend(all_referenced_vars(&scan.target.format, view)?);
            for name in names {
                owners
                    .entry(name.to_ascii_uppercase())
                    .or_default()
                    .insert(scan.target.agent.clone());
            }
        }
    }
    for row in inv.saved.iter().filter(|row| !row.agent.is_empty()) {
        let mut names = BTreeSet::new();
        collect_placeholders(&row.entry, &mut names);
        if let Some(scan) = inv.scan(&row.agent) {
            let view = entry::parse(&scan.target.format, &row.entry).map_err(|_| {
                anyhow::anyhow!("已保存 MCP 条目不可解析，无法确认密钥引用归属，拒绝操作")
            })?;
            names.extend(all_referenced_vars(&scan.target.format, &view)?);
        } else {
            // Removed registrations still reserve their saved credential references.
            // Parse all supported formats to retain bare Codex references without guessing a target.
            for format in [Format::ClaudeJson, Format::CodexToml, Format::OpenCodeJson] {
                if let Ok(view) = entry::parse(&format, &row.entry) {
                    names.extend(all_referenced_vars(&format, &view)?);
                }
            }
        }
        let row_owner = if policy
            .pointer("/agents/definitions")
            .and_then(|defs| defs.get(&row.agent))
            .is_some_and(|raw| raw.get("_delete") != Some(&Value::Bool(true)))
        {
            row.agent.clone()
        } else {
            format!("retired:{}", row.agent)
        };
        for name in names {
            owners
                .entry(name.to_ascii_uppercase())
                .or_default()
                .insert(row_owner.clone());
        }
    }
    Ok(owners)
}

/// Candidate account references, including implicit native files and disabled snapshots.
/// This first reads only the selected account so an unrelated broken account does not
/// block a credential-free official account.
pub(crate) fn account_reference_names(
    ctx: &Context,
    account: &str,
    policy: &Value,
) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    if let Some(raw) = policy
        .pointer("/agents/definitions")
        .and_then(|defs| defs.get(account))
    {
        if let Some(profile) = raw
            .get("profile")
            .filter(|_| raw.get("_delete") != Some(&Value::Bool(true)))
        {
            collect_placeholders(profile, &mut names);
            if let Some(name) = profile.get("env_key").and_then(Value::as_str) {
                names.insert(name.to_string());
            }
        }
    }
    if let Some(native_names) =
        implicit_native_mcp_refs(ctx, policy, false, Some(account))?.get(account)
    {
        names.extend(native_names.iter().cloned());
    }
    let scans = crate::native_mcp::inventory::scan_targets_checked(ctx, policy);
    if let Some(scan) = scans.iter().find(|scan| scan.target.agent == account) {
        if let Some(error) = scan.error.as_ref() {
            bail!("当前账号 MCP 配置不可解析，无法确认密钥归属: {error}");
        }
        for item in &scan.entries {
            let view = item
                .view
                .as_ref()
                .map_err(|_| anyhow::anyhow!("当前账号 MCP 条目不可解析，无法确认密钥归属"))?;
            names.extend(all_referenced_vars(&scan.target.format, view)?);
        }
    }
    let rows = crate::native_mcp::saved::load_readonly(ctx)?;
    for row in rows.iter().filter(|row| row.agent == account) {
        if let Some(scan) = scans.iter().find(|scan| scan.target.agent == account) {
            let view = entry::parse(&scan.target.format, &row.entry)
                .map_err(|_| anyhow::anyhow!("当前账号已保存 MCP 条目不可解析"))?;
            names.extend(all_referenced_vars(&scan.target.format, &view)?);
        } else {
            collect_placeholders(&row.entry, &mut names);
            for format in [Format::ClaudeJson, Format::CodexToml, Format::OpenCodeJson] {
                if let Ok(view) = entry::parse(&format, &row.entry) {
                    names.extend(all_referenced_vars(&format, &view)?);
                }
            }
        }
    }
    Ok(names)
}

/// Actual runtime references for this account only. Saved/disabled snapshots own names
/// but are not injected into a running CLI.
pub(crate) fn account_mcp_secret_names(ctx: &Context, account: &str) -> Result<BTreeSet<String>> {
    let raw = account
        .strip_prefix("profiles:")
        .or_else(|| account.strip_prefix("agents:"))
        .unwrap_or(account);
    let inv = crate::native_mcp::inventory::load(ctx)?;
    let mut names = BTreeSet::new();
    if let Some(scan) = inv.scan(raw) {
        if scan.error.is_some() {
            bail!("账号 MCP 配置不可读取或解析，拒绝启动");
        }
        for item in &scan.entries {
            let view = item
                .view
                .as_ref()
                .map_err(|_| anyhow::anyhow!("账号 MCP 条目不可解析，拒绝启动"))?;
            if !view.enabled {
                continue;
            }
            names.extend(all_referenced_vars(&scan.target.format, view)?);
        }
    }
    let policy = store::effective_policy(ctx)?;
    let native = implicit_native_mcp_refs(ctx, &policy, true, Some(raw))?;
    if let Some(native_names) = native.get(raw) {
        names.extend(native_names.iter().cloned());
    }
    validate_vars_ownership(ctx, raw, &names)?;
    Ok(names)
}

/// The same location lock protects MCP-only accounts as controlled CLI accounts.
pub(crate) fn lock_credential_account(
    ctx: &Context,
    account: &str,
    write: bool,
) -> Result<crate::native_account_context::AccountLock> {
    if account.starts_with("retired:") {
        bail!("密钥变量属于已删除或不同登记代际的账号，拒绝更换来源或修改取值；请为新账号配置新的独占变量");
    }
    let policy = store::effective_policy(ctx)?;
    let only = if account == "claude-code" || account == "codex" {
        Some(format!("agents:{account}"))
    } else if policy
        .pointer("/agents/definitions")
        .and_then(|defs| defs.get(account))
        .is_some_and(|def| def.get("profile").is_some())
    {
        Some(format!("profiles:{account}"))
    } else {
        None
    };
    let dir = if let Some(only) = only {
        crate::native_account_context::resolve_account(ctx, &only)?.dir
    } else {
        let target = crate::native_mcp::targets::agent_targets(&policy, &ctx.home_dir)
            .into_iter()
            .find(|target| target.agent == account)
            .ok_or_else(|| anyhow::anyhow!("密钥所属账号未登记，拒绝操作"))?;
        crate::native_mcp::targets::validate_target_path(&target.path)?;
        let real = crate::native_profiles::paths::validate_account_resource_path(
            ctx,
            &policy,
            account,
            &target.path,
        )?;
        crate::native_mcp::targets::resolved_path(
            real.parent()
                .ok_or_else(|| anyhow::anyhow!("账号配置目标没有目录，拒绝操作"))?,
        )
    };
    crate::native_account_context::lock_directory(ctx, &dir, write)
}

fn lock_owned_accounts(
    ctx: &Context,
    ids: &BTreeSet<String>,
) -> Result<Vec<crate::native_account_context::AccountLock>> {
    ids.iter()
        .filter(|id| !id.starts_with("retired:"))
        .map(|id| lock_credential_account(ctx, id, true))
        .collect()
}

fn collect_placeholders(value: &Value, names: &mut BTreeSet<String>) {
    match value {
        Value::String(text) => {
            let name = text
                .strip_prefix("${")
                .and_then(|s| s.strip_suffix('}'))
                .or_else(|| text.strip_prefix("{env:").and_then(|s| s.strip_suffix('}')));
            if let Some(name) = name.filter(|name| crate::secret_values::valid_var_name(name)) {
                names.insert(name.to_string());
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                collect_placeholders(value, names);
            }
        }
        Value::Array(items) => {
            for value in items {
                collect_placeholders(value, names);
            }
        }
        _ => {}
    }
}

pub(crate) fn validate_vars_ownership(
    ctx: &Context,
    account: &str,
    names: &BTreeSet<String>,
) -> Result<()> {
    if names.is_empty() {
        return Ok(());
    }
    crate::secret_values::ensure_source_ready()?;
    let owners = account_secret_owners(ctx)?;
    for name in names {
        if owners
            .get(&name.to_ascii_uppercase())
            .is_some_and(|ids| ids.iter().any(|id| id != account))
        {
            bail!("密钥变量被另一个账号引用，拒绝把该凭据带入当前账号");
        }
    }
    Ok(())
}

pub(crate) fn validate_variable_owners(
    ctx: &Context,
    name: &str,
    account: Option<&str>,
) -> Result<()> {
    let owners = account_secret_owners(ctx)?;
    let existing = owners
        .get(&name.to_ascii_uppercase())
        .cloned()
        .unwrap_or_default();
    let conflicting = match account {
        Some(id) => existing.iter().any(|owner| owner != id),
        None => existing.len() > 1 || existing.iter().any(|id| id.starts_with("retired:")),
    };
    if conflicting {
        bail!("密钥变量被不同账号共同引用，拒绝修改或应用；请为每个账号显式配置独立密钥变量");
    }
    Ok(())
}

fn status(ctx: &Context) -> Result<Value> {
    let mut variables = Vec::new();
    let mut unresolved = 0i64;
    for (server, key, kind, placeholder) in mcp_references(ctx)? {
        let resolved = crate::secret_values::is_set(&placeholder);
        if !resolved {
            unresolved += 1;
        }
        variables.push(json!({
            "server": server,
            "env_key": key,
            "placeholder": placeholder,
            "resolved": resolved,
            // 取值是在这里填的还是来自进程环境：界面据此说明改哪儿才有用
            "stored": crate::secret_values::is_stored(&placeholder),
            "kind": kind,
        }));
    }
    variables.sort_by(|a, b| {
        (
            a["server"].as_str().unwrap_or(""),
            a["env_key"].as_str().unwrap_or(""),
            a["kind"].as_str().unwrap_or(""),
        )
            .cmp(&(
                b["server"].as_str().unwrap_or(""),
                b["env_key"].as_str().unwrap_or(""),
                b["kind"].as_str().unwrap_or(""),
            ))
    });
    let (source_path, source_error) = crate::secret_values::source_state();
    let ownership = account_secret_owners(ctx);
    let ownership_conflict = ownership
        .as_ref()
        .is_ok_and(|owners| owners.values().any(|ids| ids.len() > 1));
    let retired_ownership = ownership.as_ref().is_ok_and(|owners| {
        owners
            .values()
            .any(|ids| ids.iter().any(|id| id.starts_with("retired:")))
    });
    let account_source_missing =
        source_path.is_none() && ownership.as_ref().is_ok_and(|owners| !owners.is_empty());
    let state = if source_error.is_some()
        || ownership.is_err()
        || ownership_conflict
        || retired_ownership
        || account_source_missing
    {
        "unsupported"
    } else if unresolved == 0 {
        "managed-ok"
    } else {
        "missing"
    };
    let source_ready = source_path.is_some() && source_error.is_none();
    let legacy_count = crate::secret_values::legacy_count(ctx)?;
    Ok(json!({
        "domain": "secrets",
        "elements": [{
            "id": "secrets:mcp-env",
            "domain": "secrets",
            "spec": {"sources": ["external file", "process env"]},
            "policy": {"enabled": true},
            "observed": {
                "variables": variables,
                "unresolved": unresolved,
                "source": {
                    "configured": source_path.is_some(),
                    "path": source_path,
                    "ready": source_ready,
                    "error": source_error,
                    // 文件里已有哪些变量（只有名字）：表单据此说明这个变量有没有值
                    "names": if source_ready {
                        crate::secret_values::stored_names().into_iter().collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    },
                },
                "legacy_database_values": legacy_count,
                "ownership_conflict": ownership_conflict,
                "retired_ownership": retired_ownership,
                "account_source_missing": account_source_missing,
                "ownership_error": ownership.err().map(|err| format!("{err:#}")),
                "provider_keys": provider_key_references(ctx)
                    .into_iter()
                    .map(|(var, used_by)| json!({
                        "variable": var,
                        "resolved": crate::secret_values::is_set(&var),
                        "stored": crate::secret_values::is_stored(&var),
                        "used_by": used_by,
                    }))
                    .collect::<Vec<_>>(),
            },
            "state": state,
            "freshness": {"state": "not-applicable", "current": Value::Null, "upstream": Value::Null, "checked_at": Value::Null, "probe": Value::Null, "cache_age_sec": Value::Null},
            "actions": [],
        }],
    }))
}

/// `secrets:set --var NAME --value V`：把取值写进外部文件；空值即删除该变量。
/// 错误消息只说变量种类，绝不回显取值。
fn set_value(ctx: &Context, req: &Request) -> Result<Value> {
    let mut var: Option<String> = None;
    let mut value: Option<String> = None;
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--var" => {
                if var.is_some() {
                    bail!("参数错误: --var 重复");
                }
                let Some(next) = iter.next() else {
                    bail!("参数错误: --var 缺少变量名");
                };
                if !crate::secret_values::valid_var_name(next) {
                    bail!("参数错误: 变量名不合法（须以字母或下划线开头，只含字母、数字与下划线）");
                }
                var = Some(next.clone());
            }
            "--value" => {
                if value.is_some() {
                    bail!("参数错误: --value 重复");
                }
                let Some(next) = iter.next() else {
                    bail!("参数错误: --value 缺少取值");
                };
                value = Some(next.clone());
            }
            other if other.starts_with("--") => bail!("参数错误: 存在不支持的选项"),
            _ => bail!("参数错误: 不接受位置参数"),
        }
    }
    let Some(var) = var else {
        bail!("参数错误: 缺少 --var");
    };
    let value = value.unwrap_or_default();
    if value.contains('\0') {
        bail!("参数错误: 取值含非法字符");
    }

    let owners = account_secret_owners(ctx)?;
    let ids = owners
        .get(&var.to_ascii_uppercase())
        .cloned()
        .unwrap_or_default();
    let _account_locks = lock_owned_accounts(ctx, &ids)?;
    validate_variable_owners(ctx, &var, None)?;
    let write = crate::secret_values::plan_change(ctx, &var, &value)?;
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    // Repeat the provenance check after acquiring the DB writer. Persist it before
    // source writes, including deletion, so removing a value never releases its owner.
    let current = account_secret_owners_in_policy(ctx, &store::policy_from_connection(&tx)?)?;
    let current_ids = current
        .get(&var.to_ascii_uppercase())
        .cloned()
        .unwrap_or_default();
    if current_ids != ids
        || current_ids.len() > 1
        || current_ids.iter().any(|id| id.starts_with("retired:"))
    {
        bail!("密钥引用归属已变化或存在冲突，未写入来源；请重新检查");
    }
    if let Some(account) = current_ids.iter().next() {
        claim_variable_ownership(ctx, &tx, account, &BTreeSet::from([var.clone()]))?;
    }
    write.commit(ctx)?;
    let result = write
        .verify_committed(ctx)
        .and_then(|()| tx.commit().map_err(Into::into));
    if let Err(err) = result {
        write.rollback(ctx).map_err(|_| {
            anyhow::anyhow!("密钥修改未提交，且来源发生并发变化；拒绝覆盖，请检查来源文件")
        })?;
        return Err(err.context("密钥修改未提交，来源文件已恢复"));
    }
    crate::secret_values::refresh(ctx);

    Ok(json!({
        "domain": "secrets",
        "variable": var,
        "stored": !value.is_empty(),
    }))
}

fn set_source(ctx: &Context, req: &Request) -> Result<Value> {
    let mut path: Option<String> = None;
    let mut create = false;
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--create" if !create => create = true,
            "--path" if path.is_none() => {
                path = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("参数错误: --path 缺少取值"))?
                        .trim()
                        .to_string(),
                );
            }
            "--path" => bail!("参数错误: --path 重复"),
            other if other.starts_with("--") => bail!("参数错误: 存在不支持的选项"),
            _ => bail!("参数错误: 不接受位置参数"),
        }
    }
    let path = path.ok_or_else(|| anyhow::anyhow!("参数错误: 缺少 --path"))?;
    if create && path.is_empty() {
        bail!("参数错误: --create 需要 --path");
    }
    let owners = account_secret_owners(ctx)?;
    if owners.values().any(|ids| ids.len() > 1) {
        bail!("密钥变量被不同账号共同引用，拒绝更换来源；请先显式解除冲突引用");
    }
    let ids: BTreeSet<String> = owners
        .values()
        .flat_map(|ids| ids.iter().cloned())
        .collect();
    let _account_locks = lock_owned_accounts(ctx, &ids)?;
    let created = if create {
        crate::secret_values::create_source_file(ctx, &path)?
    } else {
        None
    };
    let configured = (|| -> Result<()> {
        let source = crate::secret_values::source_identity(ctx, &path)?;
        let mut conn = store::open_write(ctx)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current = account_secret_owners_in_policy(ctx, &store::policy_from_connection(&tx)?)?;
        if current != owners {
            bail!("密钥引用归属已变化，没有更换来源；请重新检查");
        }
        for account in ids.iter().filter(|id| !id.starts_with("retired:")) {
            let names = current
                .iter()
                .filter(|(_, owners)| owners.contains(account))
                .map(|(name, _)| name.clone())
                .collect();
            claim_variable_ownership(ctx, &tx, account, &names)?;
        }
        let mut settings = store::settings_from_connection(&tx)?;
        settings.secrets_file_path = path.clone();
        store::write_settings(&tx, &settings)?;
        if crate::secret_values::source_identity(ctx, &path)? != source {
            bail!("外部密钥来源在登记时变化，没有更换来源；请重新检查");
        }
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = configured {
        if let Some(file) = created.as_ref() {
            // Only remove the unchanged empty source created by this operation.
            let marker = "# Agent Manager 保存的 API Key：一行一个 KEY=VALUE，仅本人可读\n";
            if std::fs::symlink_metadata(file)
                .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
                && std::fs::read(file).is_ok_and(|bytes| bytes == marker.as_bytes())
            {
                let _ = std::fs::remove_file(file);
            }
        }
        return Err(err);
    }
    crate::secret_values::refresh(ctx);
    Ok(json!({"domain": "secrets", "configured": !path.is_empty(), "created": created.is_some()}))
}

fn migrate_legacy(ctx: &Context, req: &Request) -> Result<Value> {
    if !req.args.is_empty() {
        bail!("secrets:migrate-legacy 不接受参数");
    }
    let owners = account_secret_owners(ctx)?;
    if owners.values().any(|ids| ids.len() > 1) {
        bail!("既有密钥变量被不同账号共同引用，拒绝迁移；原来源与数据库保持不变");
    }
    let ids = owners
        .values()
        .flat_map(|ids| ids.iter().cloned())
        .collect();
    let _account_locks = lock_owned_accounts(ctx, &ids)?;
    let migrated = crate::secret_values::migrate_legacy(ctx)?;
    crate::secret_values::refresh(ctx);
    Ok(json!({"domain": "secrets", "migrated": migrated}))
}

pub(crate) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let cmd = req.command.replace('.', ":");
    match cmd.as_str() {
        "secrets:status" => {
            let mut offline = false;
            for arg in &req.args {
                if arg == "--offline" && !offline {
                    offline = true;
                } else {
                    bail!("secrets:status 仅接受可选的单个 --offline 参数");
                }
            }
            status(ctx)
        }
        "secrets:set" => set_value(ctx, req),
        "secrets:source" => set_source(ctx, req),
        "secrets:migrate-legacy" => migrate_legacy(ctx, req),
        _ => bail!("secrets 域不支持命令 '{cmd}'"),
    }
}
