//! `profiles:seed`：把默认环境（主配置）里的共享设置一次性带进独立环境。
//!
//! 独立环境有自己的配置目录，Claude Code / Codex 不再读主配置里的用户级设置，新建的环境因此
//! 没有权限规则、状态栏、规则文件与 MCP 服务。CC Switch 在同一个目录里切换接入，这些
//! 自然都在；这里按「实际文件为准」换一种做法：用户勾选后，把主配置里对应的部分**复制一次**
//! 进实例自己的文件，之后两边各自独立。
//!
//! 规矩：
//! - 只补实例里还没有的：实例已有这一项就不动，逐项报告；
//! - 各项走各自的负责人：权限与状态栏没有别的域管，在这里写；规则与 MCP 分别交给
//!   rules / mcp 域的写入（同一套写前比对、链接解析与审计），这里只挑要写什么；
//! - 凭据一律不带：`env` 与 `apiKeyHelper` 不在可选项里；MCP 条目里有明文凭据（像凭据的 env
//!   字面量、字面量请求头、地址里内嵌的账号密码、Codex 的 `bearer_token`）的整条跳过并点名，
//!   要带就去 MCP 页逐条复制；
//! - 各项互不牵连：一项失败照常做下一项，逐项给出结果；
//! - 审计只记带了哪些项与各自结果，不记内容。

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

use super::model::{self, ProfileDef, ProfileKind};
use crate::native::{Context, Request};
use crate::native_mcp::entry::{HeaderVal, View};
use crate::store;

/// 可带的项，结果按这个顺序给出。
const ITEMS: &[&str] = &["permissions", "statusline", "rules", "mcp"];

struct SeedArgs {
    id: String,
    items: Vec<&'static str>,
}

fn parse_args(req: &Request) -> Result<SeedArgs> {
    let mut only = None;
    let mut items = None;
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        let slot = match arg.as_str() {
            "--only" => &mut only,
            "--items" => &mut items,
            other => bail!("profiles:seed 不支持参数 '{other}'"),
        };
        if slot.is_some() {
            bail!("参数 {arg} 只能给一次");
        }
        *slot = Some(
            iter.next()
                .ok_or_else(|| anyhow!("参数 {arg} 缺少取值"))?
                .clone(),
        );
    }
    let only = only.ok_or_else(|| anyhow!("缺少 --only profiles:<实例名>"))?;
    let id = only
        .strip_prefix("profiles:")
        .ok_or_else(|| anyhow!("--only 只接受 profiles:<实例名>"))?
        .to_string();
    if !model::valid_profile_id(&id) {
        bail!("实例名只能用小写字母、数字与连字符");
    }
    let raw =
        items.ok_or_else(|| anyhow!("缺少 --items（从 {} 里选，逗号分隔）", ITEMS.join("、")))?;
    Ok(SeedArgs {
        id,
        items: parse_items(&raw)?,
    })
}

pub(super) fn parse_items(raw: &str) -> Result<Vec<&'static str>> {
    let mut picked = Vec::new();
    for item in raw
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let known = ITEMS
            .iter()
            .copied()
            .find(|known| *known == item)
            .ok_or_else(|| anyhow!("不认识的项 '{item}'（只能从 {} 里选）", ITEMS.join("、")))?;
        if !picked.contains(&known) {
            picked.push(known);
        }
    }
    if picked.is_empty() {
        bail!("--items 至少选一项");
    }
    picked.sort_by_key(|item| ITEMS.iter().position(|known| known == item));
    Ok(picked)
}

pub(super) fn find_def(ctx: &Context, id: &str) -> Result<ProfileDef> {
    let policy = store::effective_policy(ctx)?;
    model::load_definition_entries(&policy)
        .into_iter()
        .find(|(entry_id, _, _)| entry_id == id)
        .ok_or_else(|| anyhow!("未知实例: {id}"))?
        .2
}

pub(super) fn config_dir(ctx: &Context, def: &ProfileDef) -> Result<PathBuf> {
    if !def.enabled {
        bail!("[{}] 实例已停用，不带设置", def.id);
    }
    model::validate_config_dir(def, &ctx.home_dir, &ctx.data_dir)
}

/// 配置目录由实例域创建：还没有就建（仅本人可访问）；已有的不改权限。
fn ensure_dir(dir: &Path) -> Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// 锁前预检：参数、实例定义与配置目录，只读零写。
pub(crate) fn preflight(ctx: &Context, req: &Request) -> Result<()> {
    let args = parse_args(req)?;
    let def = find_def(ctx, &args.id)?;
    config_dir(ctx, &def)?;
    Ok(())
}

fn result(item: &str, state: &str, detail: impl Into<String>) -> Value {
    json!({"item": item, "state": state, "detail": detail.into()})
}

fn failed(item: &str, err: &anyhow::Error) -> Value {
    result(
        item,
        "failed",
        crate::sanitize::sanitize_diagnostic_message(&format!("{err:#}")),
    )
}

fn sub_request(req: &Request, command: &str, args: Vec<String>) -> Request {
    Request {
        command: command.to_string(),
        args,
        run_id: req.run_id.clone(),
    }
}

/// 主配置里的源文件：读不到当作没有；符号链接照读最终目标。
fn read_source(path: &Path) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 8 * 1024 * 1024 {
        return None;
    }
    std::fs::read(path).ok()
}

/// 实例里的目标文件：链接解析到最终文件；不是普通文件就拒绝。
pub(super) fn read_target(path: &Path) -> Result<(PathBuf, Option<Vec<u8>>)> {
    let real = crate::native_agent_settings::resolve_write_target(path)?;
    match std::fs::symlink_metadata(&real) {
        Ok(meta) if meta.is_file() => {
            let bytes = std::fs::read(&real)?;
            Ok((real, Some(bytes)))
        }
        Ok(_) => bail!("{} 不是普通文件，拒绝读写", real.display()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok((real, None)),
        Err(err) => Err(err.into()),
    }
}

fn parse_json_object(bytes: Option<&[u8]>, what: &str) -> Result<Map<String, Value>> {
    match bytes {
        None => Ok(Map::new()),
        Some(bytes) if bytes.iter().all(u8::is_ascii_whitespace) => Ok(Map::new()),
        Some(bytes) => match serde_json::from_slice::<Value>(bytes) {
            Ok(Value::Object(map)) => Ok(map),
            _ => bail!("{what} 不是合法的 JSON 对象，没有改"),
        },
    }
}

// ---------- 权限与状态栏：没有别的域管，在这里写 ----------

/// Claude settings.json 里的两段（主配置有、实例没有才补）。
fn render_claude_settings(
    source: &Map<String, Value>,
    previous: Option<&[u8]>,
    items: &[&str],
) -> Result<(Vec<Value>, Option<Vec<u8>>)> {
    let mut target = parse_json_object(previous, "独立环境的 settings.json")?;
    let mut out = Vec::new();
    let mut changed = false;
    for item in items {
        let (key, label) = match *item {
            "permissions" => ("permissions", "权限规则"),
            _ => ("statusLine", "状态栏"),
        };
        match source.get(key) {
            None => out.push(result(item, "absent", format!("默认环境没有设置{label}"))),
            Some(_) if target.contains_key(key) => out.push(result(
                item,
                "exists",
                format!("独立环境已有{label}，没有改"),
            )),
            Some(value) => {
                target.insert(key.to_string(), value.clone());
                changed = true;
                out.push(result(item, "copied", format!("已带上{label}")));
            }
        }
    }
    let content = if changed {
        let mut bytes = serde_json::to_vec_pretty(&Value::Object(target))?;
        bytes.push(b'\n');
        Some(bytes)
    } else {
        None
    };
    Ok((out, content))
}

fn seed_claude_settings(ctx: &Context, dir: &Path, items: &[&str]) -> Result<Vec<Value>> {
    let source = parse_json_object(
        read_source(&ctx.home_dir.join(".claude/settings.json")).as_deref(),
        "默认环境的 settings.json",
    )?;
    let (real, previous) = read_target(&dir.join("settings.json"))?;
    let (out, content) = render_claude_settings(&source, previous.as_deref(), items)?;
    if let Some(bytes) = content {
        crate::native_mcp::rollback::atomic_write_bytes_checked(&real, &bytes, previous.as_deref())
            .map_err(|err| anyhow!("写 settings.json 没有成功（可能已在外部修改）：{err:#}"))?;
    }
    Ok(out)
}

/// Codex 的权限（审批、沙箱与权限档案）按一组整体带：实例里一个都没有才带，免得两边的
/// 写法拼在一起互相矛盾。键名取自 Codex `rust-v0.156.1` 的 `ConfigToml`。
pub(super) const CODEX_PERMISSION_KEYS: &[&str] = &[
    "approval_policy",
    "approvals_reviewer",
    "sandbox_mode",
    "sandbox_workspace_write",
    "default_permissions",
    "permissions",
];
/// Codex 的状态栏在 `[tui]` 下。
pub(super) const CODEX_STATUSLINE_KEYS: &[&str] = &["status_line", "status_line_use_colors"];

fn render_codex_settings(
    source: &toml_edit::Document,
    previous: Option<&[u8]>,
    items: &[&str],
) -> Result<(Vec<Value>, Option<Vec<u8>>)> {
    let mut target = previous
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default()
        .parse::<toml_edit::Document>()
        .map_err(|_| anyhow!("独立环境的 config.toml 不是合法 TOML，没有改"))?;
    let mut out = Vec::new();
    let mut changed = false;
    for item in items {
        if *item == "permissions" {
            let present: Vec<&str> = CODEX_PERMISSION_KEYS
                .iter()
                .copied()
                .filter(|key| source.get(key).is_some())
                .collect();
            if present.is_empty() {
                out.push(result(item, "absent", "默认环境没有设置审批与沙箱"));
            } else if CODEX_PERMISSION_KEYS
                .iter()
                .any(|key| target.get(key).is_some())
            {
                out.push(result(item, "exists", "独立环境已有审批或沙箱设置，没有改"));
            } else {
                for key in &present {
                    target[*key] = source[*key].clone();
                }
                changed = true;
                out.push(result(
                    item,
                    "copied",
                    format!("已带上 {}", present.join("、")),
                ));
            }
            continue;
        }
        let source_tui = source.get("tui").and_then(toml_edit::Item::as_table_like);
        let present: Vec<&str> = CODEX_STATUSLINE_KEYS
            .iter()
            .copied()
            .filter(|key| source_tui.is_some_and(|tui| tui.get(key).is_some()))
            .collect();
        let target_tui = target.get("tui");
        if present.is_empty() {
            out.push(result(item, "absent", "默认环境没有设置状态栏"));
        } else if target_tui.is_some_and(|tui| !tui.is_table_like()) {
            bail!("独立环境的 config.toml 里 tui 不是表，没有改");
        } else if target_tui
            .and_then(toml_edit::Item::as_table_like)
            .is_some_and(|tui| {
                CODEX_STATUSLINE_KEYS
                    .iter()
                    .any(|key| tui.contains_key(key))
            })
        {
            out.push(result(item, "exists", "独立环境已有状态栏设置，没有改"));
        } else {
            if target.get("tui").is_none() {
                target["tui"] = toml_edit::Item::Table(toml_edit::Table::new());
            }
            let tui = target["tui"]
                .as_table_like_mut()
                .ok_or_else(|| anyhow!("独立环境的 config.toml 里 tui 不是表，没有改"))?;
            for key in &present {
                if let Some(value) = source_tui.and_then(|src| src.get(key)) {
                    tui.insert(key, value.clone());
                }
            }
            changed = true;
            out.push(result(item, "copied", "已带上状态栏"));
        }
    }
    let content = if changed {
        let content = target.to_string();
        let parsed: toml::Value = content
            .parse()
            .map_err(|_| anyhow!("合并后的 config.toml 无法解析，没有写入"))?;
        let fatal = crate::native_agent_settings::codex_fatal_issues(&parsed);
        if !fatal.is_empty() {
            bail!(
                "带上后的 config.toml 会让 Codex 拒绝加载，没有写入：{}",
                fatal.join("；")
            );
        }
        Some(content.into_bytes())
    } else {
        None
    };
    Ok((out, content))
}

fn seed_codex_settings(ctx: &Context, dir: &Path, items: &[&str]) -> Result<Vec<Value>> {
    let source = match read_source(&ctx.home_dir.join(".codex/config.toml")) {
        Some(bytes) => String::from_utf8_lossy(&bytes)
            .parse::<toml_edit::Document>()
            .map_err(|_| anyhow!("默认环境的 config.toml 不是合法 TOML，没有带"))?,
        None => toml_edit::Document::new(),
    };
    let (real, previous) = read_target(&dir.join("config.toml"))?;
    let (out, content) = render_codex_settings(&source, previous.as_deref(), items)?;
    if let Some(bytes) = content {
        crate::native_mcp::rollback::atomic_write_bytes_checked(&real, &bytes, previous.as_deref())
            .map_err(|err| anyhow!("写 config.toml 没有成功（可能已在外部修改）：{err:#}"))?;
    }
    Ok(out)
}

/// 在实例主文件写入之后，按相同合并规则计算已勾选的权限与状态栏；只读、不解析凭据。
pub(super) fn preview_settings(
    ctx: &Context,
    kind: ProfileKind,
    after_profile: Vec<u8>,
    items: &[&str],
) -> Result<(Vec<u8>, Vec<Value>)> {
    let selected: Vec<&str> = items
        .iter()
        .copied()
        .filter(|item| matches!(*item, "permissions" | "statusline"))
        .collect();
    if selected.is_empty() {
        return Ok((after_profile, Vec::new()));
    }
    let (results, rendered) = match kind {
        ProfileKind::Claude => {
            let source = parse_json_object(
                read_source(&ctx.home_dir.join(".claude/settings.json")).as_deref(),
                "默认环境的 settings.json",
            )?;
            render_claude_settings(&source, Some(&after_profile), &selected)?
        }
        ProfileKind::Codex => {
            let source = match read_source(&ctx.home_dir.join(".codex/config.toml")) {
                Some(bytes) => String::from_utf8_lossy(&bytes)
                    .parse::<toml_edit::Document>()
                    .map_err(|_| anyhow!("默认环境的 config.toml 不是合法 TOML，没有带"))?,
                None => toml_edit::Document::new(),
            };
            render_codex_settings(&source, Some(&after_profile), &selected)?
        }
    };
    Ok((rendered.unwrap_or(after_profile), results))
}

// ---------- 规则：交给规则域写 ----------

pub(super) fn element<'a>(status: &'a Value, id: &str) -> Option<&'a Value> {
    status
        .get("elements")?
        .as_array()?
        .iter()
        .find(|el| el.get("id").and_then(Value::as_str) == Some(id))
}

fn blocked(el: &Value) -> Option<&str> {
    el.pointer("/observed/blocked_reason")
        .and_then(Value::as_str)
}

fn seed_rules(ctx: &Context, req: &Request, def: &ProfileDef) -> Result<Value> {
    let status = crate::native_rules::handle(ctx, &sub_request(req, "rules:status", vec![]))?;
    let (main_id, file) = match def.kind {
        ProfileKind::Claude => ("rules:claude_global", "CLAUDE.md"),
        ProfileKind::Codex => ("rules:codex_global", "AGENTS.md"),
    };
    let target_id = format!("rules:profile:{}", def.id);
    let main = element(&status, main_id).ok_or_else(|| anyhow!("默认环境没有规则目标"))?;
    let content = main.pointer("/observed/content").and_then(Value::as_str);
    if content.is_none_or(|text| text.trim().is_empty()) {
        return Ok(match blocked(main) {
            Some(why) => result("rules", "absent", format!("读不到默认环境的 {file}：{why}")),
            None => result("rules", "absent", format!("默认环境没有 {file}")),
        });
    }
    let target = element(&status, &target_id).ok_or_else(|| anyhow!("找不到这个环境的规则目标"))?;
    if let Some(reason) = blocked(target) {
        bail!("{reason}");
    }
    let existing = target.pointer("/observed/content").and_then(Value::as_str);
    if existing.is_some_and(|text| !text.trim().is_empty()) {
        return Ok(result(
            "rules",
            "exists",
            format!("独立环境已有 {file}，没有改"),
        ));
    }
    let base = target
        .pointer("/observed/base")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("这个环境的 {file} 当前不能读写"))?;
    crate::native_rules::handle(
        ctx,
        &sub_request(
            req,
            "rules:apply",
            vec![
                "--only".into(),
                target_id,
                "--base".into(),
                base.to_string(),
                "--from".into(),
                main_id.to_string(),
            ],
        ),
    )?;
    Ok(result("rules", "copied", format!("已复制 {file}")))
}

// ---------- MCP：逐条交给 mcp 域的「复制到」 ----------

/// 条目里有没有明文凭据：像凭据的 env 字面量、字面量请求头、地址里内嵌的账号密码，
/// 或 Codex 的 `bearer_token`。
pub(super) fn has_literal_credentials(format: &crate::native_mcp::Format, view: &View) -> bool {
    let literal_env = view.env.iter().any(|(key, value)| {
        crate::native_mcp::entry::env_ref(format, value).is_none()
            && (crate::native::credential_like_name(key)
                || crate::native::reject_secret_text(value).is_some())
    });
    let literal_header = view
        .headers
        .iter()
        .any(|(_, value)| matches!(value, HeaderVal::Lit(_)));
    literal_env
        || literal_header
        || crate::native::url_has_userinfo(&view.url)
        || view.extra.contains_key("bearer_token")
}

fn seed_mcp(ctx: &Context, req: &Request, def: &ProfileDef) -> Result<Value> {
    let inv = crate::native_mcp::inventory::load(ctx)?;
    let main_agent = crate::native_mcp::model::canonicalize_agent_id(&def.base_agent);
    let Some(source) = inv.scan(&main_agent) else {
        return Ok(result("mcp", "absent", "默认环境没有 MCP 配置"));
    };
    if let Some(err) = &source.error {
        return Ok(result(
            "mcp",
            "absent",
            format!("读不到默认环境的 MCP 配置：{err}"),
        ));
    }
    if source.entries.is_empty() {
        return Ok(result("mcp", "absent", "默认环境没有 MCP 服务"));
    }
    let target = inv
        .scan(&def.id)
        .ok_or_else(|| anyhow!("找不到这个环境的 MCP 配置目标"))?;
    if let Some(err) = &target.error {
        bail!("{err}");
    }
    let format = source.target.format.clone();
    let mut todo = Vec::new();
    let (mut existing, mut skipped) = (Vec::new(), Vec::new());
    for entry in &source.entries {
        if target.entry(&entry.name).is_some() || inv.saved_row(&entry.name, &def.id).is_some() {
            existing.push(entry.name.clone());
            continue;
        }
        match &entry.view {
            Ok(view) if !has_literal_credentials(&format, view) => {
                todo.push((entry.name.clone(), entry.base.clone()))
            }
            _ => skipped.push(entry.name.clone()),
        }
    }
    let (mut copied, mut failures, mut notes) = (Vec::new(), Vec::new(), Vec::new());
    for (name, base) in todo {
        let outcome = crate::native_mcp::dispatch(
            ctx,
            &sub_request(
                req,
                "mcp:apply",
                vec![
                    "--only".into(),
                    format!("mcp:{name}"),
                    "--action".into(),
                    "copy".into(),
                    "--agent".into(),
                    main_agent.clone(),
                    "--to".into(),
                    def.id.clone(),
                    "--base".into(),
                    base,
                ],
            ),
        );
        match outcome {
            Ok((_, warnings)) => {
                notes.extend(warnings);
                copied.push(name);
            }
            Err(err) => failures.push(format!(
                "{name}（{}）",
                crate::sanitize::sanitize_diagnostic_message(&format!("{err:#}"))
            )),
        }
    }
    let mut parts = Vec::new();
    if !copied.is_empty() {
        parts.push(format!("已复制 {}", copied.join("、")));
    }
    if !existing.is_empty() {
        parts.push(format!("已有同名、没有改：{}", existing.join("、")));
    }
    if !skipped.is_empty() {
        parts.push(format!(
            "含明文凭据或格式不认识、没有复制：{}（需要的话在 MCP 页逐条复制）",
            skipped.join("、")
        ));
    }
    if !failures.is_empty() {
        parts.push(format!("没有复制成功：{}", failures.join("；")));
    }
    parts.extend(notes);
    let state = if !copied.is_empty() {
        "copied"
    } else if !failures.is_empty() {
        "failed"
    } else if !skipped.is_empty() {
        "skipped"
    } else {
        "exists"
    };
    Ok(json!({
        "item": "mcp",
        "state": state,
        "detail": parts.join("；"),
        "skipped": skipped,
    }))
}

/// `profiles:seed --only profiles:<实例> --items permissions,statusline,rules,mcp`（取锁）。
pub(crate) fn seed(ctx: &Context, req: &Request) -> Result<Value> {
    let args = parse_args(req)?;
    let def = find_def(ctx, &args.id)?;
    let dir = config_dir(ctx, &def)?;
    ensure_dir(&dir)?;

    let mut results = Vec::new();
    let settings_items: Vec<&str> = args
        .items
        .iter()
        .copied()
        .filter(|item| matches!(*item, "permissions" | "statusline"))
        .collect();
    if !settings_items.is_empty() {
        let outcome = match def.kind {
            ProfileKind::Claude => seed_claude_settings(ctx, &dir, &settings_items),
            ProfileKind::Codex => seed_codex_settings(ctx, &dir, &settings_items),
        };
        match outcome {
            Ok(items) => results.extend(items),
            Err(err) => results.extend(settings_items.iter().map(|item| failed(item, &err))),
        }
    }
    for item in &args.items {
        let outcome = match *item {
            "rules" => seed_rules(ctx, req, &def),
            "mcp" => seed_mcp(ctx, req, &def),
            _ => continue,
        };
        results.push(outcome.unwrap_or_else(|err| failed(item, &err)));
    }

    if let Ok(mut conn) = store::open_write(ctx) {
        if let Ok(tx) = conn.transaction() {
            let _ = store::record_operation(
                &tx,
                Some(&req.run_id),
                "profiles:seed",
                Some(&args.id),
                Some(&json!({
                    "items": results
                        .iter()
                        .map(|r| json!({"item": r["item"], "state": r["state"]}))
                        .collect::<Vec<_>>(),
                })),
            );
            let _ = tx.commit();
        }
    }

    Ok(json!({
        "domain": "profiles",
        "action": "seed",
        "element": format!("profiles:{}", args.id),
        "results": results,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_mcp::Format;

    fn req(args: &[&str]) -> Request {
        Request {
            command: "profiles:seed".into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            run_id: "t".into(),
        }
    }

    #[test]
    fn parses_items_in_fixed_order_and_dedups() {
        let parsed = parse_args(&req(&[
            "--only",
            "profiles:cc-work",
            "--items",
            "mcp, permissions,mcp",
        ]))
        .unwrap();
        assert_eq!(parsed.id, "cc-work");
        assert_eq!(parsed.items, vec!["permissions", "mcp"]);
    }

    #[test]
    fn rejects_bad_args() {
        for args in [
            vec!["--items", "mcp"],
            vec!["--only", "cc-work", "--items", "mcp"],
            vec!["--only", "profiles:Bad", "--items", "mcp"],
            vec!["--only", "profiles:cc-work"],
            vec!["--only", "profiles:cc-work", "--items", ""],
            vec!["--only", "profiles:cc-work", "--items", "env"],
            vec!["--only", "profiles:cc-work", "--items", "apiKeyHelper"],
            vec![
                "--only",
                "profiles:a",
                "--only",
                "profiles:b",
                "--items",
                "mcp",
            ],
            vec!["--only", "profiles:cc-work", "--items", "mcp", "--force"],
            vec!["--only"],
        ] {
            assert!(parse_args(&req(&args)).is_err(), "{args:?} 应拒绝");
        }
    }

    fn view(env: &[(&str, &str)], headers: Vec<(&str, HeaderVal)>, url: &str) -> View {
        View {
            transport: crate::native_mcp::entry::Transport::Stdio,
            command: "npx".into(),
            args: vec![],
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            url: url.into(),
            headers: headers
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            startup_timeout_sec: None,
            enabled: true,
            legacy_marker: false,
            extra: Map::new(),
        }
    }

    #[test]
    fn literal_credentials_are_detected_only_for_literals() {
        let claude = Format::ClaudeJson;
        let referenced = view(
            &[("GITHUB_TOKEN", "${GITHUB_TOKEN}"), ("LOG", "debug")],
            vec![("Authorization", HeaderVal::Ref("TOKEN".into()))],
            "https://mcp.example.com/sse",
        );
        assert!(!has_literal_credentials(&claude, &referenced));
        let literal_env = view(&[("GITHUB_TOKEN", "not-a-reference")], vec![], "");
        assert!(has_literal_credentials(&claude, &literal_env));
        let literal_header = view(
            &[],
            vec![("Authorization", HeaderVal::Lit("Bearer x".into()))],
            "",
        );
        assert!(has_literal_credentials(&claude, &literal_header));
        let userinfo = view(&[], vec![], "https://user:pass@mcp.example.com/sse");
        assert!(has_literal_credentials(&claude, &userinfo));
        let mut bearer = view(&[], vec![], "https://mcp.example.com");
        bearer.extra.insert("bearer_token".into(), json!("x"));
        assert!(has_literal_credentials(&Format::CodexToml, &bearer));
    }
}
