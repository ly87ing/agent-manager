//! 「已保存未生效」的 MCP 项：SQLite `mcp_saved` 表。
//!
//! 两类行：
//! - `agent` 非空：该助手的停用快照（Claude 停用时移出的条目，或旧库迁移时已分配但
//!   实际不存在/已停用的条目），`entry` 为该助手的原生形态；
//! - `agent` 为空：未分配到任何助手的已保存定义（旧库迁移、`policy:set` 或配置备份
//!   导入），`entry` 为中立定义 {command,args,env,startup_timeout_sec,extra_args_by_agent}
//!   或 {url,headers}。
//!
//! 不变式（写入时强制，读取时违反的行显示为无效项）：headers 只存引用；env 只存
//! `${VAR}`（或 OpenCode `{env:VAR}`）引用与未命中疑似凭据检测的普通字面量；整行不得
//! 含疑似凭据。`resolve` 只列写回时要把 `${VAR}` 解析成取值的键名。

use anyhow::{bail, Result};
use rusqlite::Connection;
use serde_json::{json, Map, Value};

use super::model::{
    canonicalize_agent_id, digest_of, is_valid_server_name, now_iso, placeholder_var,
};
use crate::native::Context;
use crate::store;

pub const TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS mcp_saved (
    server TEXT NOT NULL,
    agent TEXT NOT NULL,
    entry_json TEXT NOT NULL,
    resolve_json TEXT NOT NULL,
    origin TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (server, agent)
)";

pub const ORIGIN_DISABLED: &str = "disabled";
pub const ORIGIN_MIGRATED: &str = "migrated";
pub const ORIGIN_IMPORTED: &str = "imported";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resolve {
    pub env: Vec<String>,
    pub headers: Vec<String>,
}

impl Resolve {
    pub fn to_json(&self) -> Value {
        json!({"env": self.env, "headers": self.headers})
    }

    pub fn from_json(value: &Value) -> Result<Self> {
        let list = |key: &str| -> Result<Vec<String>> {
            match value.get(key) {
                None | Some(Value::Null) => Ok(Vec::new()),
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| anyhow::anyhow!("resolve.{key} 只能含字符串"))
                    })
                    .collect(),
                Some(_) => bail!("resolve.{key} 必须是数组"),
            }
        };
        if !value.is_object() {
            bail!("resolve 必须是对象");
        }
        Ok(Self {
            env: list("env")?,
            headers: list("headers")?,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SavedRow {
    pub server: String,
    /// 规范 agent id；空串 = 未分配。
    pub agent: String,
    pub entry: Value,
    pub resolve: Resolve,
    pub origin: String,
}

impl SavedRow {
    pub fn base(&self) -> String {
        digest_of(&json!({"entry": self.entry, "resolve": self.resolve.to_json()}))
    }

    /// 配置备份里的形态（只含引用、变量名与普通字面量）。
    pub fn to_backup(&self) -> Value {
        json!({
            "server": self.server,
            "agent": self.agent,
            "entry": self.entry,
            "resolve": self.resolve.to_json(),
            "origin": self.origin,
        })
    }

    pub fn from_backup(value: &Value) -> Result<Self> {
        let obj = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("mcp_saved 的每一项必须是对象"))?;
        let text = |key: &str| -> Result<String> {
            obj.get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| anyhow::anyhow!("mcp_saved 项缺少字符串字段 {key}"))
        };
        let row = Self {
            server: text("server")?,
            agent: text("agent")?,
            entry: obj.get("entry").cloned().unwrap_or(Value::Null),
            resolve: Resolve::from_json(obj.get("resolve").unwrap_or(&json!({})))?,
            origin: text("origin").unwrap_or_else(|_| ORIGIN_IMPORTED.to_string()),
        };
        validate_row(&row)?;
        Ok(row)
    }
}

fn is_reference(value: &str) -> bool {
    placeholder_var(value).is_some()
        || value
            .strip_prefix("{env:")
            .and_then(|rest| rest.strip_suffix('}'))
            .map_or(false, crate::secret_values::valid_var_name)
}

fn check_env_values(map: Option<&Value>, what: &str) -> Result<()> {
    let Some(map) = map else { return Ok(()) };
    let map = map
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{what} 必须是对象"))?;
    for (key, value) in map {
        let text = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("{what}.{key} 必须是字符串"))?;
        if !is_reference(text)
            && (crate::native::credential_like_name(key)
                || crate::native::reject_secret_text(text).is_some())
        {
            bail!("已保存项的 {what}.{key} 含疑似凭据明文，拒绝保存");
        }
    }
    Ok(())
}

fn check_header_refs(map: Option<&Value>, what: &str) -> Result<()> {
    let Some(map) = map else { return Ok(()) };
    let map = map
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{what} 必须是对象"))?;
    for (key, value) in map {
        let text = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("{what}.{key} 必须是字符串"))?;
        if !is_reference(text) {
            bail!("已保存项的 {what}.{key} 不是变量引用，拒绝保存明文");
        }
    }
    Ok(())
}

/// 行不变式校验。错误消息不回显取值。
pub fn validate_row(row: &SavedRow) -> Result<()> {
    if !is_valid_server_name(&row.server) {
        bail!("已保存项的服务名非法: '{}'", row.server);
    }
    if !row.agent.is_empty() && canonicalize_agent_id(&row.agent) != row.agent {
        bail!("已保存项的助手 id 非规范: '{}'", row.agent);
    }
    let obj = row
        .entry
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("已保存项 {} 的条目必须是对象", row.server))?;
    if row.agent.is_empty() {
        let def = &row.entry;
        super::policy_view::validate_definition(def, &row.server)?;
        super::policy_view::validate_definition_env(def)?;
        super::policy_view::validate_definition_headers(def)?;
    } else {
        check_env_values(obj.get("env"), "env")?;
        check_env_values(obj.get("environment"), "environment")?;
        check_header_refs(obj.get("headers"), "headers")?;
        if obj
            .get("http_headers")
            .and_then(Value::as_object)
            .map_or(false, |m| !m.is_empty())
        {
            bail!("已保存项不能含 http_headers 字面量");
        }
    }
    crate::native::reject_secrets_value(&row.entry)
        .map_err(|_| anyhow::anyhow!("已保存项 {} 含疑似凭据明文，拒绝保存", row.server))?;
    let env = obj.get("env").or_else(|| obj.get("environment"));
    for key in &row.resolve.env {
        let ok = env
            .and_then(|m| m.get(key))
            .and_then(Value::as_str)
            .map_or(false, |v| placeholder_var(v).is_some());
        if !ok {
            bail!(
                "已保存项 {} 的 resolve.env 引用了不存在的变量键 {key}",
                row.server
            );
        }
    }
    for key in &row.resolve.headers {
        let ok = obj
            .get("headers")
            .and_then(|m| m.get(key))
            .and_then(Value::as_str)
            .map_or(false, |v| placeholder_var(v).is_some());
        if !ok {
            bail!(
                "已保存项 {} 的 resolve.headers 引用了不存在的 header {key}",
                row.server
            );
        }
    }
    Ok(())
}

fn missing_table(err: &rusqlite::Error) -> bool {
    err.to_string().contains("no such table")
}

/// 读取全部行；缺表按空处理。结构损坏（JSON 解析失败）fail-closed。
pub fn load_from(conn: &Connection) -> Result<Vec<SavedRow>> {
    let mut stmt = match conn.prepare(
        "SELECT server, agent, entry_json, resolve_json, origin FROM mcp_saved ORDER BY server, agent",
    ) {
        Ok(stmt) => stmt,
        Err(err) if missing_table(&err) => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (server, agent, entry_json, resolve_json, origin) = row?;
        let entry: Value = serde_json::from_str(&entry_json)
            .map_err(|_| anyhow::anyhow!("mcp_saved 中 {server} 的条目 JSON 损坏，拒绝读取"))?;
        let resolve_value: Value = serde_json::from_str(&resolve_json)
            .map_err(|_| anyhow::anyhow!("mcp_saved 中 {server} 的 resolve JSON 损坏，拒绝读取"))?;
        out.push(SavedRow {
            server,
            agent,
            entry,
            resolve: Resolve::from_json(&resolve_value)?,
            origin,
        });
    }
    Ok(out)
}

pub fn load_readonly(ctx: &Context) -> Result<Vec<SavedRow>> {
    match store::open_readonly(ctx)? {
        Some(conn) => load_from(&conn),
        None => Ok(Vec::new()),
    }
}

/// Validate every imported snapshot against the candidate policy and the full
/// candidate reference graph before committing any configuration or DB rows.
pub(crate) fn validate_saved_rows(ctx: &Context, policy: &Value, rows: &[SavedRow]) -> Result<()> {
    let mut owners = crate::native_secrets::account_secret_owners_in_policy(ctx, policy)?;
    let targets = super::targets::agent_targets(policy, &ctx.home_dir);
    let mut candidate = Vec::new();
    for row in rows {
        validate_row(row)?;
        if row.agent.is_empty() {
            continue;
        }
        let target = targets
            .iter()
            .find(|target| target.agent == row.agent)
            .ok_or_else(|| {
                anyhow::anyhow!("已保存 MCP 设置的账号 {} 未登记，没有导入", row.agent)
            })?;
        let view = super::entry::parse(&target.format, &row.entry).map_err(|_| {
            anyhow::anyhow!(
                "已保存 MCP 设置无法确认账号 {} 的有效引用，没有导入",
                row.agent
            )
        })?;
        let names = super::entry::all_referenced_vars(&target.format, &view)?;
        for name in &names {
            owners
                .entry(name.to_ascii_uppercase())
                .or_default()
                .insert(row.agent.clone());
        }
        candidate.push((row.agent.as_str(), names));
    }
    for (agent, names) in candidate {
        for name in names {
            if !crate::secret_values::valid_var_name(&name) {
                bail!("已保存 MCP 设置的变量名无效，没有导入");
            }
            if owners
                .get(&name.to_ascii_uppercase())
                .is_some_and(|accounts| accounts.iter().any(|owner| owner != agent))
            {
                bail!("变量 {name} 已属于其他账号，拒绝导入跨账号 MCP 引用");
            }
        }
    }
    Ok(())
}

pub fn ensure_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(TABLE_DDL)?;
    Ok(())
}

pub fn upsert(conn: &Connection, row: &SavedRow) -> Result<()> {
    validate_row(row)?;
    conn.execute(
        "INSERT INTO mcp_saved (server, agent, entry_json, resolve_json, origin, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(server, agent) DO UPDATE SET entry_json = excluded.entry_json,
             resolve_json = excluded.resolve_json, origin = excluded.origin,
             updated_at = excluded.updated_at",
        rusqlite::params![
            row.server,
            row.agent,
            serde_json::to_string(&row.entry)?,
            serde_json::to_string(&row.resolve.to_json())?,
            row.origin,
            now_iso()
        ],
    )?;
    Ok(())
}

pub fn exists(conn: &Connection, server: &str, agent: &str) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT count(*) FROM mcp_saved WHERE server = ?1 AND agent = ?2",
        rusqlite::params![server, agent],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

pub fn delete(conn: &Connection, server: &str, agent: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM mcp_saved WHERE server = ?1 AND agent = ?2",
        rusqlite::params![server, agent],
    )?;
    Ok(())
}

/// 中立定义去掉库时代的非定义字段（`agents`、`enabled`、`_*`）。
pub fn neutral_definition(def: &Value) -> Value {
    let mut out = Map::new();
    if let Some(map) = def.as_object() {
        for (key, value) in map {
            if key.starts_with('_') || key == "enabled" || key == "agents" || key == "type" {
                continue;
            }
            out.insert(key.clone(), value.clone());
        }
    }
    Value::Object(out)
}
