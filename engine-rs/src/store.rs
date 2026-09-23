//! SQLite backing store for the native core.
//!
//! - Database file: `<data_dir>/native-manager.db` (explicitly a NEW store; no
//!   migration from or compatibility with the legacy Python manager.db).
//! - Shared schema (this module owns it): schema_meta, policy_entries,
//!   operation_logs. Resource domains create their own tables; read paths must
//!   treat a missing domain table as empty and never create tables.
//! - `open_readonly` never creates directories or files: missing DB -> Ok(None).
//! - `open_write` creates the data dir, initializes the shared schema and
//!   freezes the builtin defaults into policy_entries on first write
//!   ("firstwrite固化"), so later reads see a stable persisted baseline.
//!   On an existing store it also runs write-path data migrations inside the
//!   same write transaction (retire_unadopted_builtin_skill_repo); read paths
//!   derive the same outcome on the fly and never migrate.
//! - Write paths run on ONE connection with an explicit transaction and a busy
//!   timeout; there is no nested/double initialization (schema init happens in
//!   open_write before any user transaction starts).
//!
//! Builtin defaults are compiled in read-only: only the two reviewer-verified
//! official assistant definitions (no machine-specific config, no secrets, no
//! uninstall commands without verified evidence).

use std::path::PathBuf;

use anyhow::{anyhow, bail, Context as _, Result};
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::native::Context;

pub const DB_FILE_NAME: &str = "native-manager.db";
pub const SCHEMA_VERSION: i64 = 1;
pub const DEFAULT_LOG_RETENTION_DAYS: u32 = 30;
pub const MAX_LOG_RETENTION_DAYS: u32 = 3650;

fn default_log_retention_days() -> u32 {
    DEFAULT_LOG_RETENTION_DAYS
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppSettings {
    #[serde(default = "default_log_retention_days")]
    pub log_retention_days: u32,
    /// 外部私有凭据文件路径；只存路径，凭据取值永不进入本数据库。
    #[serde(default)]
    pub secrets_file_path: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            log_retention_days: DEFAULT_LOG_RETENTION_DAYS,
            secrets_file_path: String::new(),
        }
    }
}

/// `usage.pricing` 是美元/百万 token 的单价，取自 Anthropic 官方定价页
/// （platform.claude.com/docs/en/about-claude/pricing，2026-09-20 核对）：
/// 缓存写入为输入价的 1.25 倍、缓存读取为 0.1 倍。只内置官方模型——
/// 第三方与中转站的价格各家各时不同，编一个比不报更糟，留给用户自己填。
/// 按前缀匹配，因此带日期后缀的模型 id 也能命中。
///
/// Reviewer-verified official defaults (docs: claude.ai/install.sh native
/// install + manual `claude update`; chatgpt.com/codex/install.sh for both
/// install and update). Field shape keeps `providers[preferred_provider].
/// install/upgrade/uninstall` so the GUI needs no restructuring; uninstall is
/// omitted (no verified official command).
pub const BUILTIN_POLICY_JSON: &str = r#"{
  "agents": {
    "claude-code": {"enabled": true},
    "codex": {"enabled": true},
    "definitions": {
    "claude-code": {
      "display_name": "Claude Code", "bin": "claude", "skills_dir": "~/.claude/skills", "mcp_config": {"path": "~/.claude.json", "format": "json"},
      "upstream_npm": "@anthropic-ai/claude-code",
      "preferred_provider": "native",
      "providers": {
        "native": {
          "install": "curl -fsSL https://claude.ai/install.sh | bash",
          "upgrade": "claude update", "managed": true
        }
      }
    },
    "codex": {
      "display_name": "Codex", "bin": "codex", "skills_dir": "~/.codex/skills", "mcp_config": {"path": "~/.codex/config.toml", "format": "toml"},
      "upstream_npm": "@openai/codex",
      "preferred_provider": "native",
      "providers": {
        "native": {
          "install": "curl -fsSL https://chatgpt.com/codex/install.sh | sh",
          "upgrade": "curl -fsSL https://chatgpt.com/codex/install.sh | sh", "managed": true
        }
      }
    }
      }
    },
  "mcp": {"definitions": {}},
  "usage": {
    "pricing": {
      "claude-opus-5": {"input": 5, "output": 25, "cache_write": 6.25, "cache_read": 0.5},
      "claude-sonnet-5": {"input": 2, "output": 10, "cache_write": 2.5, "cache_read": 0.2},
      "claude-haiku-4-5": {"input": 1, "output": 5, "cache_write": 1.25, "cache_read": 0.1}
    }
  },
  "skills": {"repositories": {}}
}"#;

pub fn builtin_policy() -> Value {
    serde_json::from_str(BUILTIN_POLICY_JSON).expect("builtin policy template is valid JSON")
}

/// 旧版本内置并在首次写库时固化的技能集合名。现在它只是编译内置的推荐来源
/// （native_skills::recommend），不再进库。
pub const LEGACY_BUILTIN_SKILL_COLLECTION: &str = "agent-skills";

/// 旧版本各时期固化进库的 `skills.repositories.agent-skills` 原值（按提交历史逐字取出：
/// 77491f5 起 6 项 + 两个助手、676ad59 起多一个 opencode、947bc09 起 7 项）。
const LEGACY_BUILTIN_SKILL_REPOS: &[&str] = &[
    r#"{"url": "https://github.com/ly87ing/agent-skills.git",
        "skill_paths": ["safe-merge-review", "change-discipline", "frontend-verification",
                        "artifact-hygiene", "reader-facing-writing", "technical-diagramming"],
        "agents": ["claude-code", "codex"]}"#,
    r#"{"url": "https://github.com/ly87ing/agent-skills.git",
        "skill_paths": ["safe-merge-review", "change-discipline", "frontend-verification",
                        "artifact-hygiene", "reader-facing-writing", "technical-diagramming"],
        "agents": ["claude-code", "codex", "opencode"]}"#,
    r#"{"url": "https://github.com/ly87ing/agent-skills.git",
        "skill_paths": ["safe-merge-review", "solution-shaping", "change-discipline",
                        "verification", "artifact-hygiene", "reader-facing-writing",
                        "technical-diagramming"],
        "agents": ["claude-code", "codex"]}"#,
];

/// 集合定义是否仍是旧版本固化的内置原值（逐字段相同）。只比对定义本身；
/// 是否从未生效（无 skill_bindings）由调用方按各自的数据源判定。
pub fn is_legacy_builtin_skill_repo(name: &str, value: &Value) -> bool {
    name == LEGACY_BUILTIN_SKILL_COLLECTION
        && LEGACY_BUILTIN_SKILL_REPOS.iter().any(|raw| {
            serde_json::from_str::<Value>(raw).expect("legacy builtin skill repo is valid JSON")
                == *value
        })
}

/// 写路径迁移：旧库里固化、仍为原值且从未在任何助手生效（该集合没有任何
/// skill_bindings 行）的内置集合，从 policy 移除（连同它的集合开关）并记操作日志。
/// 有 bindings 或被改过的保持为用户集合不动；已下载的仓库镜像不删。幂等：
/// 条件不成立即零写。只在 open_write 的写事务里调用，读路径按同一判定即时推导。
fn retire_unadopted_builtin_skill_repo(conn: &Connection) -> Result<()> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value_json FROM policy_entries WHERE key = 'skills'",
            [],
            |row| row.get(0),
        )
        .ok();
    let Some(raw) = raw else {
        return Ok(());
    };
    let Ok(mut skills) = serde_json::from_str::<Value>(&raw) else {
        // 损坏的 skills 行交给读路径 fail-closed 报错，迁移不碰它
        return Ok(());
    };
    let name = LEGACY_BUILTIN_SKILL_COLLECTION;
    let Some(repo) = skills.get("repositories").and_then(|repos| repos.get(name)) else {
        return Ok(());
    };
    if !is_legacy_builtin_skill_repo(name, repo) {
        return Ok(());
    }
    let has_bindings_table: bool = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='skill_bindings'",
        [],
        |row| row.get::<_, i64>(0),
    )? > 0;
    if has_bindings_table {
        let bindings: i64 = conn.query_row(
            "SELECT count(*) FROM skill_bindings WHERE collection = ?1",
            [name],
            |row| row.get(0),
        )?;
        if bindings > 0 {
            return Ok(());
        }
    }
    let object = skills
        .as_object_mut()
        .ok_or_else(|| anyhow!("skills 配置不是对象"))?;
    object.remove(name);
    if let Some(repos) = object
        .get_mut("repositories")
        .and_then(Value::as_object_mut)
    {
        repos.remove(name);
    }
    conn.execute(
        "UPDATE policy_entries SET value_json = ?1, updated_at = ?2 WHERE key = 'skills'",
        rusqlite::params![serde_json::to_string(&skills)?, now_iso()],
    )?;
    record_operation(
        conn,
        None,
        "skills:retire-builtin",
        Some(name),
        Some(&serde_json::json!({
            "action": "retire-builtin",
            "reason": "旧版本固化的内置技能仓库从未在任何助手生效，改为推荐来源",
        })),
    )?;
    Ok(())
}

/// Path of the native database (pure: never touches the filesystem).
pub fn db_path(ctx: &Context) -> PathBuf {
    ctx.data_dir.join(DB_FILE_NAME)
}

fn configure_connection(conn: &Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    Ok(())
}

/// Read-only connection. Missing DB -> Ok(None); existing but foreign/corrupt
/// (no shared schema_meta) -> Err, fail-closed without modifying the file.
/// Never creates directories, files, or tables.
pub fn open_readonly(ctx: &Context) -> Result<Option<Connection>> {
    let path = db_path(ctx);
    if !path.exists() {
        return Ok(None);
    }
    if !path.is_file() || path.is_symlink() {
        bail!("数据库路径不是普通文件，拒绝访问: {}", path.display());
    }
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("无法打开数据库 {}", path.display()))?;
    configure_connection(&conn)?;
    let has_schema_meta: bool = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='schema_meta'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .unwrap_or(false);
    if !has_schema_meta {
        bail!(
            "数据库缺少共享 schema_meta 表 ({})，拒绝读取以保护原数据",
            path.display()
        );
    }
    Ok(Some(conn))
}

/// upstream_probes 表 DDL（system:refresh-upstream 探测缓存的唯一存储）。
/// 与各自域表同一纪律：只由写路径（refresh-upstream 的 Immediate 事务）经
/// ensure_upstream_probes 幂等建表；status 只读，缺表按空缓存处理，绝不建表。
pub const UPSTREAM_PROBES_DDL: &str = "CREATE TABLE IF NOT EXISTS upstream_probes (
    element_id TEXT PRIMARY KEY,
    probe TEXT,
    state TEXT NOT NULL,
    current TEXT,
    upstream TEXT,
    error TEXT,
    checked_at TEXT NOT NULL
);";

/// 建表迁移：幂等，供 refresh-upstream 写路径在任何既有库上补齐该表。
pub fn ensure_upstream_probes(conn: &Connection) -> Result<()> {
    conn.execute_batch(UPSTREAM_PROBES_DDL)?;
    Ok(())
}

/// Shared schema. Domain tables are created by their owners, not here.
fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS schema_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS policy_entries (
            key TEXT PRIMARY KEY,
            value_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS operation_logs (
            id TEXT PRIMARY KEY,
            run_id TEXT,
            operation_type TEXT NOT NULL,
            target TEXT,
            detail_json TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS app_settings (
            key TEXT PRIMARY KEY,
            value_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        ",
    )?;
    Ok(())
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Write connection: creates the data dir (0700 on unix), initializes the
/// shared schema once, and freezes the builtin defaults into policy_entries
/// when the store is first created. WAL + busy_timeout for cross-process
/// safety. The single returned connection is the only write handle — callers
/// run their transaction on it directly.
pub fn open_write(ctx: &Context) -> Result<Connection> {
    let path = db_path(ctx);
    if ctx.data_dir.exists() && (ctx.data_dir.is_symlink() || !ctx.data_dir.is_dir()) {
        bail!("数据目录不是普通目录，拒绝访问: {}", ctx.data_dir.display());
    }
    if path.exists() && (!path.is_file() || path.is_symlink()) {
        bail!("数据库路径不是普通文件，拒绝访问: {}", path.display());
    }
    let existing = path.is_file();
    let data_dir_existed = ctx.data_dir.exists();
    std::fs::create_dir_all(&ctx.data_dir)
        .with_context(|| format!("无法创建数据目录 {}", ctx.data_dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !data_dir_existed {
            std::fs::set_permissions(&ctx.data_dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let mut conn =
        Connection::open(&path).with_context(|| format!("无法打开数据库 {}", path.display()))?;
    #[cfg(unix)]
    if !existing {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    configure_connection(&conn)?;
    if !existing {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // 另一个进程可能在本进程检查 path 后已经完成初始化；锁内复查避免重复插入。
        let already_initialized: bool = tx
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='schema_meta'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if already_initialized {
            tx.commit()?;
            return Ok(conn);
        }
        init_schema(&tx)?;
        let now = now_iso();
        tx.execute(
            "INSERT INTO schema_meta (key, value) VALUES ('schema_version', ?1)",
            [SCHEMA_VERSION.to_string()],
        )?;
        tx.execute(
            "INSERT INTO schema_meta (key, value) VALUES ('initialized_at', ?1)",
            [now.clone()],
        )?;
        // firstwrite 固化：内置默认进入 policy_entries，之后读取以库内为准
        write_policy(&tx, &builtin_policy())?;
        tx.execute(
            "INSERT INTO operation_logs (id, run_id, operation_type, target, detail_json, created_at)
             VALUES (?1, NULL, 'db:init', NULL, NULL, ?2)",
            [uuid_v4(), now],
        )?;
        tx.commit()?;
    } else {
        // BEGIN IMMEDIATE 等待并发首次初始化提交后再验证；不会把半成品误判为外来库。
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: Option<String> = tx
            .query_row(
                "SELECT value FROM schema_meta WHERE key='schema_version'",
                [],
                |row| row.get(0),
            )
            .ok();
        match version.and_then(|raw| raw.parse::<i64>().ok()) {
            Some(ver) if ver <= SCHEMA_VERSION => {
                // 旧库数据迁移只在写路径执行，与版本校验同一写事务
                retire_unadopted_builtin_skill_repo(&tx)?;
                tx.commit()?
            }
            Some(ver) => {
                tx.rollback()?;
                bail!("数据库版本 ({ver}) 高于当前程序支持版本 ({SCHEMA_VERSION})，拒绝访问")
            }
            None => {
                tx.rollback()?;
                bail!(
                    "数据库缺少 schema_version 记录 ({})，拒绝写入以保护原数据",
                    path.display()
                )
            }
        }
    }
    Ok(conn)
}

pub fn default_settings() -> AppSettings {
    AppSettings::default()
}

pub fn settings_from_connection(conn: &Connection) -> Result<AppSettings> {
    let mut settings = default_settings();
    let mut stmt = conn.prepare("SELECT key, value_json FROM app_settings")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (key, raw) = row?;
        // 已退役偏好留下的行：只跳过（零写）。default_dry_run 随公共 plan/dry-run
        // 退役；offline 与 upstream_ttl_sec 是没有消费者的旋钮，已从界面撤下。
        // 其他未知行仍由 deny_unknown_fields fail closed。
        if matches!(
            key.as_str(),
            "default_dry_run" | "offline" | "upstream_ttl_sec"
        ) {
            continue;
        }
        let mut object = serde_json::to_value(&settings)?
            .as_object()
            .cloned()
            .unwrap();
        object.insert(key, serde_json::from_str(&raw)?);
        settings = serde_json::from_value(Value::Object(object))?;
    }
    Ok(settings)
}

pub fn effective_settings(ctx: &Context) -> Result<AppSettings> {
    match open_readonly(ctx)? {
        Some(conn) => settings_from_connection(&conn),
        None => Ok(default_settings()),
    }
}

pub fn write_settings(conn: &Connection, settings: &AppSettings) -> Result<()> {
    if settings.log_retention_days > MAX_LOG_RETENTION_DAYS {
        bail!(
            "日志保留天数超出允许范围 (0..={MAX_LOG_RETENTION_DAYS}): {}",
            settings.log_retention_days
        );
    }
    if settings.secrets_file_path.contains('\0') || settings.secrets_file_path.len() > 4096 {
        bail!("外部密钥文件路径不合法");
    }
    let object = serde_json::to_value(settings)?
        .as_object()
        .cloned()
        .unwrap();
    // 序列化后的完整对象先校验，之后才开始写第一行。
    let validated: AppSettings = serde_json::from_value(Value::Object(object.clone()))?;
    let values = serde_json::to_value(validated)?
        .as_object()
        .cloned()
        .unwrap();
    // 校验通过后顺带清除已退役偏好留下的行；与调用方同一事务边界。
    conn.execute(
        "DELETE FROM app_settings WHERE key IN ('default_dry_run', 'offline', 'upstream_ttl_sec')",
        [],
    )?;
    for (key, item) in values {
        conn.execute("INSERT INTO app_settings (key, value_json, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json, updated_at=excluded.updated_at", rusqlite::params![key, serde_json::to_string(&item)?, now_iso()])?;
    }
    Ok(())
}

/// Rebuild the policy tree from policy_entries (one row per top-level key).
/// Shared schema only; missing rows simply mean an empty tree.
pub fn policy_from_connection(conn: &Connection) -> Result<Value> {
    let mut stmt = conn.prepare("SELECT key, value_json FROM policy_entries")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut map = Map::new();
    for row in rows {
        let (key, raw) = row?;
        let value: Value = serde_json::from_str(&raw)
            .with_context(|| format!("配置项 {key} 的 JSON 数据损坏，拒绝输出半份配置"))?;
        map.insert(key, value);
    }
    Ok(Value::Object(map))
}

/// Effective policy: persisted store when the DB exists (builtin defaults were
/// frozen into it at first write); builtin defaults when it does not.
pub fn effective_policy(ctx: &Context) -> Result<Value> {
    match open_readonly(ctx)? {
        Some(conn) => policy_from_connection(&conn),
        None => Ok(builtin_policy()),
    }
}

/// Replace the whole policy tree with `value` (object; one row per top-level
/// key). Callers pass `&transaction` to compose with other writes atomically —
/// this function never opens its own transaction.
pub fn write_policy(conn: &Connection, value: &Value) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("策略必须是 JSON 对象"))?;
    let now = now_iso();
    conn.execute("DELETE FROM policy_entries", [])?;
    for (key, item) in object {
        let raw = serde_json::to_string(item)?;
        conn.execute(
            "INSERT INTO policy_entries (key, value_json, updated_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![key, raw, now],
        )?;
    }
    Ok(())
}

pub fn uuid_v4() -> String {
    // RFC 4122 v4 from /dev/urandom; no external dependency.
    let mut bytes = [0u8; 16];
    let filled = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
        .is_ok();
    if !filled {
        // 兜底：时间戳+PID 混合（仅用于日志 id，非安全场景）
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for (i, chunk) in bytes.chunks_mut(8).enumerate() {
            chunk.copy_from_slice(
                &(nanos
                    .wrapping_add((std::process::id() as u128) << 64)
                    .wrapping_add(i as u128))
                .to_le_bytes()[..8],
            );
        }
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

pub fn prune_operation_logs(conn: &Connection) -> Result<usize> {
    let settings = settings_from_connection(conn)?;
    if settings.log_retention_days == 0 {
        return Ok(0);
    }
    let modifier = format!("-{} days", settings.log_retention_days);
    let deleted = conn.execute(
        "DELETE FROM operation_logs WHERE datetime(created_at) < datetime('now', ?1)",
        rusqlite::params![modifier],
    )?;
    Ok(deleted)
}

pub fn clear_operation_logs(conn: &Connection) -> Result<usize> {
    let deleted = conn.execute("DELETE FROM operation_logs", [])?;
    Ok(deleted)
}

/// Append one audit entry. Never stores secret-bearing payloads for sources;
/// callers choose the detail. Automatically prunes logs older than retention threshold.
pub fn record_operation(
    conn: &Connection,
    run_id: Option<&str>,
    operation_type: &str,
    target: Option<&str>,
    detail: Option<&Value>,
) -> Result<()> {
    conn.execute_batch("SAVEPOINT am_record_op;")?;
    let run = || -> Result<()> {
        let detail_json = detail.map(serde_json::to_string).transpose()?;
        conn.execute(
            "INSERT INTO operation_logs (id, run_id, operation_type, target, detail_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                uuid_v4(),
                run_id,
                operation_type,
                target,
                detail_json,
                now_iso()
            ],
        )?;
        prune_operation_logs(conn)?;
        Ok(())
    };
    match run() {
        Ok(()) => {
            conn.execute_batch("RELEASE SAVEPOINT am_record_op;")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT am_record_op; RELEASE SAVEPOINT am_record_op;",
            );
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::Context;

    fn ctx(name: &str) -> (std::path::PathBuf, Context) {
        let root =
            std::env::temp_dir().join(format!("am-settings-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&root);
        (
            root.clone(),
            Context {
                data_dir: root.join("data"),
                home_dir: root.join("home"),
            },
        )
    }

    #[test]
    fn legacy_builtin_skill_repo_matches_only_historical_values() {
        for raw in LEGACY_BUILTIN_SKILL_REPOS {
            let value: Value = serde_json::from_str(raw).unwrap();
            assert!(is_legacy_builtin_skill_repo("agent-skills", &value));
            // 同值换了集合名不算
            assert!(!is_legacy_builtin_skill_repo("my-skills", &value));
            // 任一字段被改过（含界面保存留下的 _delete:false）都是用户集合
            let mut edited = value.clone();
            edited["_delete"] = Value::Bool(false);
            assert!(!is_legacy_builtin_skill_repo("agent-skills", &edited));
            let mut edited = value.clone();
            edited["agents"] = serde_json::json!(["codex"]);
            assert!(!is_legacy_builtin_skill_repo("agent-skills", &edited));
        }
        // 当前内置 policy 不再带任何技能集合
        assert_eq!(
            builtin_policy()["skills"],
            serde_json::json!({"repositories": {}})
        );
    }

    #[test]
    fn settings_default_read_is_non_mutating() {
        let (root, context) = ctx("default");
        assert_eq!(
            effective_settings(&context).unwrap(),
            AppSettings::default()
        );
        assert_eq!(AppSettings::default().log_retention_days, 30);
        assert!(AppSettings::default().secrets_file_path.is_empty());
        assert!(!context.data_dir.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn settings_round_trip_and_serde_rejects_without_partial_write() {
        let (root, context) = ctx("roundtrip");
        let conn = open_write(&context).unwrap();
        let settings = AppSettings {
            log_retention_days: 14,
            secrets_file_path: "~/private/secrets.env".into(),
        };
        let tx = conn.unchecked_transaction().unwrap();
        write_settings(&tx, &settings).unwrap();
        tx.commit().unwrap();
        drop(conn);
        assert_eq!(effective_settings(&context).unwrap(), settings);
        let conn = open_write(&context).unwrap();
        let before: String = conn
            .query_row(
                "SELECT group_concat(key || '=' || value_json, ',') FROM app_settings",
                [],
                |r| r.get(0),
            )
            .unwrap();
        for raw in [
            r#"{"log_retention_days":-1}"#,
            r#"{"log_retention_days":20,"extra":true}"#,
            // 已退役的偏好不再是字段：整对象层面同样 fail closed（行级跳过见下一个用例）
            r#"{"log_retention_days":20,"offline":false}"#,
            r#"{"log_retention_days":20,"upstream_ttl_sec":1}"#,
            r#"{"log_retention_days":20,"default_dry_run":false}"#,
        ] {
            assert!(serde_json::from_str::<AppSettings>(raw).is_err());
        }
        let parsed_snake: AppSettings =
            serde_json::from_str(r#"{"log_retention_days":20}"#).unwrap();
        assert_eq!(parsed_snake.log_retention_days, 20);

        let mut invalid = settings.clone();
        invalid.log_retention_days = 3651;
        assert!(write_settings(&conn, &invalid).is_err());

        let after: String = conn
            .query_row(
                "SELECT group_concat(key || '=' || value_json, ',') FROM app_settings",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(before, after);
        let _ = std::fs::remove_dir_all(root);
    }

    /// 在 app_settings 里直接种入指定行（模拟旧版本写入的 legacy/未知行）。
    fn seed_settings_row(conn: &Connection, key: &str, value_json: &str) {
        conn.execute(
            "INSERT INTO app_settings (key, value_json, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json, updated_at=excluded.updated_at",
            rusqlite::params![key, value_json, now_iso()],
        )
        .unwrap();
    }

    #[test]
    fn settings_legacy_rows_are_skipped_with_zero_writes() {
        let (root, context) = ctx("legacy-skip");
        let conn = open_write(&context).unwrap();
        // 旧版本写下的三种已退役偏好行都要被跳过，而不是让整库 fail closed
        seed_settings_row(&conn, "offline", "true");
        seed_settings_row(&conn, "upstream_ttl_sec", "123");
        seed_settings_row(&conn, "default_dry_run", "true");
        seed_settings_row(&conn, "log_retention_days", "20");
        drop(conn);

        let db = db_path(&context);
        let bytes_before = std::fs::read(&db).unwrap();
        let mtime_before = std::fs::metadata(&db).unwrap().modified().unwrap();

        let settings = effective_settings(&context).unwrap();
        assert_eq!(settings.log_retention_days, 20, "其他合法行仍须生效");

        let bytes_after = std::fs::read(&db).unwrap();
        let mtime_after = std::fs::metadata(&db).unwrap().modified().unwrap();
        assert_eq!(bytes_before, bytes_after, "读取 legacy 行不得写库");
        assert_eq!(mtime_before, mtime_after, "读取 legacy 行不得触动文件");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn write_settings_removes_legacy_rows() {
        let (root, context) = ctx("legacy-purge");
        let conn = open_write(&context).unwrap();
        seed_settings_row(&conn, "default_dry_run", "true");
        seed_settings_row(&conn, "offline", "false");
        seed_settings_row(&conn, "upstream_ttl_sec", "42");

        let settings = AppSettings {
            log_retention_days: 7,
            ..AppSettings::default()
        };
        let tx = conn.unchecked_transaction().unwrap();
        write_settings(&tx, &settings).unwrap();
        tx.commit().unwrap();

        let legacy_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM app_settings WHERE key IN ('default_dry_run', 'offline', 'upstream_ttl_sec')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(legacy_count, 0, "保存后 legacy 行必须消失");
        drop(conn);
        assert_eq!(
            effective_settings(&context).unwrap(),
            settings,
            "保存后其余值必须完整正确"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn settings_unknown_row_still_fails_closed() {
        let (root, context) = ctx("unknown-row");
        let conn = open_write(&context).unwrap();
        seed_settings_row(&conn, "mystery_key", "true");
        drop(conn);

        let db = db_path(&context);
        let bytes_before = std::fs::read(&db).unwrap();
        assert!(
            effective_settings(&context).is_err(),
            "未知行必须 fail closed"
        );
        let bytes_after = std::fs::read(&db).unwrap();
        assert_eq!(bytes_before, bytes_after, "fail closed 不得写库");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn auto_retention_pruning_and_manual_clear() {
        let (root, context) = ctx("retention");
        let conn = open_write(&context).unwrap();
        let mut settings = default_settings();
        settings.log_retention_days = 10;
        write_settings(&conn, &settings).unwrap();

        // 插入粗粒度过期记录（15天前）和有效记录（2天前）
        conn.execute(
            "INSERT INTO operation_logs (id, run_id, operation_type, created_at)
             VALUES ('old-1', 'run-old', 'test:old', datetime('now', '-15 days'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO operation_logs (id, run_id, operation_type, created_at)
             VALUES ('recent-1', 'run-recent', 'test:recent', datetime('now', '-2 days'))",
            [],
        )
        .unwrap();

        // 插入精确秒级边界记录：10天阈值外 1 秒（删除）与阈值内 1 秒（保留）
        conn.execute(
            "INSERT INTO operation_logs (id, run_id, operation_type, created_at)
             VALUES ('boundary-expired', 'run-boundary', 'test:expired', datetime('now', '-10 days', '-1 second'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO operation_logs (id, run_id, operation_type, created_at)
             VALUES ('boundary-retained', 'run-boundary', 'test:retained', datetime('now', '-10 days', '+1 second'))",
            [],
        )
        .unwrap();

        // 写入新记录触发自动清理
        record_operation(&conn, Some("run-new"), "test:new", None, None).unwrap();

        let count_old: i64 = conn
            .query_row(
                "SELECT count(*) FROM operation_logs WHERE id = 'old-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count_old, 0, "15天前的记录应已被自动清理");
        let count_recent: i64 = conn
            .query_row(
                "SELECT count(*) FROM operation_logs WHERE id = 'recent-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count_recent, 1, "2天前的记录应保留");

        let count_expired: i64 = conn
            .query_row(
                "SELECT count(*) FROM operation_logs WHERE id = 'boundary-expired'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count_expired, 0, "阈值外 1 秒的记录应已被自动清理");

        let count_retained: i64 = conn
            .query_row(
                "SELECT count(*) FROM operation_logs WHERE id = 'boundary-retained'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count_retained, 1, "阈值内 1 秒的记录应被保留");

        // 设置为 0：不自动清理
        settings.log_retention_days = 0;
        write_settings(&conn, &settings).unwrap();
        conn.execute(
            "INSERT INTO operation_logs (id, run_id, operation_type, created_at)
             VALUES ('old-2', 'run-old-2', 'test:old2', datetime('now', '-100 days'))",
            [],
        )
        .unwrap();
        record_operation(&conn, Some("run-new-2"), "test:new2", None, None).unwrap();
        let count_old2: i64 = conn
            .query_row(
                "SELECT count(*) FROM operation_logs WHERE id = 'old-2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count_old2, 1, "0 保留天数时不应自动清理过期记录");

        // 验证清理失败导致写入事务失败（fail-closed）
        settings.log_retention_days = 5;
        write_settings(&conn, &settings).unwrap();
        conn.execute(
            "CREATE TRIGGER fail_prune BEFORE DELETE ON operation_logs
             BEGIN SELECT RAISE(ABORT, 'trigger blocked delete'); END;",
            [],
        )
        .unwrap();
        let err = record_operation(&conn, Some("run-fail"), "test:fail", None, None);
        assert!(err.is_err(), "清理失败须导致当前写入失败");
        let count_fail: i64 = conn
            .query_row(
                "SELECT count(*) FROM operation_logs WHERE run_id = 'run-fail'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count_fail, 0, "写入失败不得遗留记录");
        conn.execute("DROP TRIGGER fail_prune", []).unwrap();

        // 验证 clear_operation_logs
        let cleared = clear_operation_logs(&conn).unwrap();
        assert!(cleared > 0, "应清空现有记录");
        let total_count: i64 = conn
            .query_row("SELECT count(*) FROM operation_logs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total_count, 0, "operation_logs 应全空");

        let _ = std::fs::remove_dir_all(root);
    }
}
