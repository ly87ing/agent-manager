//! 凭据取值只来自用户显式选择的外部 `.env` 文件或进程环境。
//!
//! `native-manager.db` 只保存外部文件路径，不保存凭据值。外部文件优先于进程环境，
//! 这样从 Dock / Finder 启动时也能稳定解析，同时仍保留终端注入作为回退。

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use rusqlite::Connection;

use crate::native::Context;

#[derive(Clone, Default)]
struct Snapshot {
    values: BTreeMap<String, String>,
    configured_path: Option<String>,
    error: Option<String>,
}

thread_local! {
    static CACHE: RefCell<Snapshot> = RefCell::new(Snapshot::default());
}

/// 变量名规则：与 `${VAR}` 占位符可用的形状一致。
pub fn valid_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn configured_path(ctx: &Context) -> Result<Option<(String, PathBuf)>> {
    let raw = crate::store::effective_settings(ctx)?.secrets_file_path;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let path = if trimmed == "~" {
        ctx.home_dir.clone()
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        ctx.home_dir.join(rest)
    } else {
        PathBuf::from(trimmed)
    };
    if !path.is_absolute() {
        bail!("外部密钥文件必须使用绝对路径或 ~/ 开头的路径");
    }
    Ok(Some((trimmed.to_string(), path)))
}

#[cfg(unix)]
fn validate_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(path)?.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        bail!("外部密钥文件权限过宽，请先执行 chmod 600");
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

fn validated_source_path(ctx: &Context) -> Result<(String, PathBuf)> {
    let Some((display, path)) = configured_path(ctx)? else {
        bail!("尚未选择外部密钥文件，请先到“设置 → 密钥”配置");
    };
    let metadata = fs::symlink_metadata(&path).with_context(|| "外部密钥文件不存在或无法读取")?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("外部密钥文件必须是普通文件，不能是目录或符号链接");
    }
    validate_permissions(&path)?;
    let canonical = path
        .canonicalize()
        .with_context(|| "无法解析外部密钥文件路径")?;
    if let Ok(data_dir) = ctx.data_dir.canonicalize() {
        if canonical.starts_with(data_dir) {
            bail!("外部密钥文件不能放在 Agent Manager 数据目录中");
        }
    }
    Ok((display, canonical))
}

/// 新建一个空的外部密钥文件（`secrets:source --create`）：文件已存在就什么都不动；不存在时
/// 缺的父目录以 0700 新建，文件以 0600 独占新建（不跟随、不覆盖）。不能建在数据目录里。
/// 返回这次新建的文件路径。
pub fn create_source_file(ctx: &Context, raw: &str) -> Result<Option<PathBuf>> {
    let trimmed = raw.trim();
    let path = if let Some(rest) = trimmed.strip_prefix("~/") {
        ctx.home_dir.join(rest)
    } else {
        PathBuf::from(trimmed)
    };
    if !path.is_absolute() || trimmed == "~" {
        bail!("外部密钥文件必须使用绝对路径或 ~/ 开头的文件路径");
    }
    if path.starts_with(&ctx.data_dir) {
        bail!("外部密钥文件不能放在 Agent Manager 数据目录中");
    }
    if fs::symlink_metadata(&path).is_ok() {
        return Ok(None);
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("外部密钥文件路径没有上级目录"))?;
    if !parent.exists() {
        fs::create_dir_all(parent).with_context(|| "无法创建外部密钥文件所在的目录")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        }
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .with_context(|| "无法新建外部密钥文件")?;
    file.write_all("# Agent Manager 保存的 API Key：一行一个 KEY=VALUE，仅本人可读\n".as_bytes())?;
    file.sync_all()?;
    Ok(Some(path))
}

pub fn validate_source(ctx: &Context, raw: &str) -> Result<()> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let path = if trimmed == "~" {
        ctx.home_dir.clone()
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        ctx.home_dir.join(rest)
    } else {
        PathBuf::from(trimmed)
    };
    if !path.is_absolute() {
        bail!("外部密钥文件必须使用绝对路径或 ~/ 开头的路径");
    }
    let metadata = fs::symlink_metadata(&path).with_context(|| "外部密钥文件不存在或无法读取")?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("外部密钥文件必须是普通文件，不能是目录或符号链接");
    }
    validate_permissions(&path)?;
    let canonical = path
        .canonicalize()
        .with_context(|| "无法解析外部密钥文件路径")?;
    if let Ok(data_dir) = ctx.data_dir.canonicalize() {
        if canonical.starts_with(data_dir) {
            bail!("外部密钥文件不能放在 Agent Manager 数据目录中");
        }
    }
    let text = fs::read_to_string(&path).with_context(|| "外部密钥文件不是可读的 UTF-8 文本")?;
    parse_env(&text)?;
    Ok(())
}

fn parse_value(raw: &str) -> Result<String> {
    let value = raw.trim();
    if value.starts_with('"') {
        return serde_json::from_str::<String>(value)
            .map_err(|_| anyhow::anyhow!("双引号取值格式不正确"));
    }
    if value.starts_with('\'') {
        if value.len() < 2 || !value.ends_with('\'') {
            bail!("单引号取值格式不正确");
        }
        return Ok(value[1..value.len() - 1].to_string());
    }
    Ok(value.to_string())
}

fn key_of_line(line: &str) -> Option<&str> {
    let mut item = line.trim();
    if item.is_empty() || item.starts_with('#') {
        return None;
    }
    if let Some(rest) = item.strip_prefix("export ") {
        item = rest.trim_start();
    }
    let (key, _) = item.split_once('=')?;
    let key = key.trim();
    valid_var_name(key).then_some(key)
}

fn parse_env(text: &str) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let mut item = line.trim();
        if item.is_empty() || item.starts_with('#') {
            continue;
        }
        if let Some(rest) = item.strip_prefix("export ") {
            item = rest.trim_start();
        }
        let Some((key, raw)) = item.split_once('=') else {
            bail!("外部密钥文件第 {} 行不是 KEY=VALUE", index + 1);
        };
        let key = key.trim();
        if !valid_var_name(key) {
            bail!("外部密钥文件第 {} 行的变量名不合法", index + 1);
        }
        let value =
            parse_value(raw).with_context(|| format!("外部密钥文件第 {} 行取值无效", index + 1))?;
        if value.contains('\0') {
            bail!("外部密钥文件第 {} 行含非法字符", index + 1);
        }
        if values.insert(key.to_string(), value).is_some() {
            bail!("外部密钥文件中存在重复变量 {key}");
        }
    }
    Ok(values)
}

fn load_snapshot(ctx: &Context) -> Snapshot {
    let configured = match configured_path(ctx) {
        Ok(value) => value,
        Err(err) => {
            return Snapshot {
                error: Some(format!("{err:#}")),
                ..Snapshot::default()
            };
        }
    };
    let Some((display, _)) = configured else {
        return Snapshot::default();
    };
    match validated_source_path(ctx).and_then(|(_, path)| {
        let text = fs::read_to_string(path).with_context(|| "外部密钥文件不是可读的 UTF-8 文本")?;
        parse_env(&text)
    }) {
        Ok(values) => Snapshot {
            values,
            configured_path: Some(display),
            error: None,
        },
        Err(err) => Snapshot {
            values: BTreeMap::new(),
            configured_path: Some(display),
            error: Some(format!("{err:#}")),
        },
    }
}

/// 每条命令开头重新读取外部文件；用户修改后无需重启应用。
pub fn refresh(ctx: &Context) {
    CACHE.with(|cache| *cache.borrow_mut() = load_snapshot(ctx));
}

pub fn lookup(name: &str) -> Option<String> {
    let from_file = CACHE.with(|cache| {
        cache
            .borrow()
            .values
            .get(name)
            .filter(|value| !value.is_empty())
            .cloned()
    });
    from_file.or_else(|| std::env::var(name).ok().filter(|value| !value.is_empty()))
}

pub fn is_set(name: &str) -> bool {
    lookup(name).is_some()
}

/// 变量是否来自已选择的外部密钥文件。
pub fn is_stored(name: &str) -> bool {
    CACHE.with(|cache| {
        cache
            .borrow()
            .values
            .get(name)
            .is_some_and(|value| !value.is_empty())
    })
}

pub fn source_state() -> (Option<String>, Option<String>) {
    CACHE.with(|cache| {
        let snapshot = cache.borrow();
        (snapshot.configured_path.clone(), snapshot.error.clone())
    })
}

/// 外部密钥文件里已有的变量名（含空值）；只返回名字。
pub fn stored_names() -> BTreeSet<String> {
    CACHE.with(|cache| cache.borrow().values.keys().cloned().collect())
}

fn render_line(name: &str, value: &str) -> Result<String> {
    Ok(format!("{name}={}", serde_json::to_string(value)?))
}

fn render_updated(text: &str, updates: &BTreeMap<String, Option<String>>) -> Result<String> {
    let mut seen = BTreeSet::new();
    let mut lines = Vec::new();
    for line in text.lines() {
        if let Some(key) = key_of_line(line) {
            if let Some(value) = updates.get(key) {
                seen.insert(key.to_string());
                if let Some(value) = value {
                    lines.push(render_line(key, value)?);
                }
                continue;
            }
        }
        lines.push(line.to_string());
    }
    for (key, value) in updates {
        if !seen.contains(key) {
            if let Some(value) = value {
                lines.push(render_line(key, value)?);
            }
        }
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    Ok(out)
}

fn atomic_write(path: &Path, text: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("外部密钥文件没有父目录"))?;
    let temp = parent.join(format!(
        ".agent-manager-secrets-{}-{}.tmp",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<()> {
        let mut file = options.open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.with_context(|| "写入外部密钥文件失败")
}

fn update_values(ctx: &Context, updates: &BTreeMap<String, Option<String>>) -> Result<()> {
    let (_, path) = validated_source_path(ctx)?;
    let text = fs::read_to_string(&path).with_context(|| "外部密钥文件不是可读的 UTF-8 文本")?;
    parse_env(&text)?;
    let updated = render_updated(&text, updates)?;
    atomic_write(&path, &updated)
}

pub fn set(ctx: &Context, name: &str, value: &str) -> Result<()> {
    if !valid_var_name(name) {
        bail!("变量名不合法：要以字母或下划线开头，只能用字母、数字与下划线");
    }
    let mut updates = BTreeMap::new();
    updates.insert(
        name.to_string(),
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        },
    );
    update_values(ctx, &updates)
}

/// 计算好、还没落盘的一次密钥文件写入。
struct PlannedWrite {
    path: PathBuf,
    new: Vec<u8>,
}

/// 只补缺失值的合并结果：同名不同值时拒绝；没有要补的返回 None。不落盘。
fn plan_merge_named(
    ctx: &Context,
    incoming: &BTreeMap<String, String>,
) -> Result<(Option<PlannedWrite>, Vec<String>)> {
    let (_, path) = validated_source_path(ctx)?;
    let text = fs::read_to_string(&path).with_context(|| "外部密钥文件不是可读的 UTF-8 文本")?;
    let current = parse_env(&text)?;
    let mut updates = BTreeMap::new();
    let mut merged = Vec::new();
    for (name, value) in incoming {
        if !valid_var_name(name) {
            continue;
        }
        match current.get(name) {
            Some(existing) if existing.is_empty() => {
                updates.insert(name.clone(), Some(value.clone()));
            }
            Some(existing) if existing != value => {
                bail!("外部密钥文件中的变量 {name} 已存在且取值不同，拒绝覆盖")
            }
            Some(_) => {}
            None => {
                updates.insert(name.clone(), Some(value.clone()));
            }
        }
        merged.push(name.clone());
    }
    merged.sort();
    merged.dedup();
    if updates.is_empty() {
        return Ok((None, merged));
    }
    let updated = render_updated(&text, &updates)?;
    Ok((
        Some(PlannedWrite {
            path,
            new: updated.into_bytes(),
        }),
        merged,
    ))
}

/// 只补缺失值；同名不同值时拒绝，避免接管覆盖用户在外部仓库维护的凭据。
pub fn merge_without_overwrite(
    ctx: &Context,
    incoming: &BTreeMap<String, String>,
) -> Result<Vec<String>> {
    let (write, merged) = plan_merge_named(ctx, incoming)?;
    if let Some(write) = write {
        let text = String::from_utf8(write.new).with_context(|| "外部密钥文件不是 UTF-8 文本")?;
        atomic_write(&write.path, &text)?;
    }
    Ok(merged)
}

pub fn legacy_count(ctx: &Context) -> Result<usize> {
    let Some(conn) = crate::store::open_readonly(ctx)? else {
        return Ok(0);
    };
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'secret_values'",
        [],
        |row| row.get(0),
    )?;
    if exists == 0 {
        return Ok(0);
    }
    Ok(
        conn.query_row("SELECT COUNT(*) FROM secret_values", [], |row| {
            row.get::<_, i64>(0)
        })? as usize,
    )
}

fn read_legacy(conn: &Connection) -> Result<BTreeMap<String, String>> {
    let mut stmt = conn.prepare("SELECT name, value FROM secret_values ORDER BY name")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (name, value) = row?;
        out.insert(name, value);
    }
    Ok(out)
}

/// 显式迁移：先无覆盖写入外部文件，成功后才删除旧表，失败时数据库原样保留。
pub fn migrate_legacy(ctx: &Context) -> Result<Vec<String>> {
    let Some(conn) = crate::store::open_readonly(ctx)? else {
        return Ok(Vec::new());
    };
    let count = legacy_count(ctx)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let values = read_legacy(&conn)?;
    drop(conn);
    let migrated = merge_without_overwrite(ctx, &values)?;
    let conn = crate::store::open_write(ctx)?;
    conn.execute("DROP TABLE IF EXISTS secret_values", [])?;
    Ok(migrated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn var_name_rules_match_placeholder_shape() {
        assert!(valid_var_name("GROK_API_KEY"));
        assert!(valid_var_name("_x1"));
        assert!(!valid_var_name(""));
        assert!(!valid_var_name("1BAD"));
        assert!(!valid_var_name("has-dash"));
        assert!(!valid_var_name("has space"));
    }

    #[test]
    fn env_parser_supports_quotes_and_rejects_duplicates() {
        let parsed = parse_env("# comment\nA=plain\nB='json value'\nC=\"line\\nvalue\"\n").unwrap();
        assert_eq!(parsed.get("A").map(String::as_str), Some("plain"));
        assert_eq!(parsed.get("B").map(String::as_str), Some("json value"));
        assert_eq!(parsed.get("C").map(String::as_str), Some("line\nvalue"));
        assert!(parse_env("A=one\nA=two\n").is_err());
        assert!(parse_env("bad-name=value\n").is_err());
    }

    #[test]
    fn updater_preserves_unowned_lines_and_can_remove() {
        let mut updates = BTreeMap::new();
        updates.insert("A".into(), Some("new value".into()));
        updates.insert("C".into(), Some("three".into()));
        updates.insert("B".into(), None);
        let rendered = render_updated("# keep\nA=old\nB=two\n", &updates).unwrap();
        assert!(rendered.starts_with("# keep\n"));
        assert!(rendered.contains("A=\"new value\"\n"));
        assert!(!rendered.contains("B="));
        assert!(rendered.contains("C=\"three\"\n"));
    }

    #[test]
    fn empty_file_value_is_not_reported_as_a_configured_secret() {
        CACHE.with(|cache| {
            *cache.borrow_mut() = Snapshot {
                values: BTreeMap::from([("EMPTY_TEST_SECRET".into(), String::new())]),
                ..Snapshot::default()
            };
        });
        assert_eq!(lookup("EMPTY_TEST_SECRET"), None);
        assert!(!is_set("EMPTY_TEST_SECRET"));
        assert!(!is_stored("EMPTY_TEST_SECRET"));
    }
}
