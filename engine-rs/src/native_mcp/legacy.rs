//! 旧库（「服务库定义」`policy.mcp.definitions` + `mcp_bindings` 分配表）到
//! 「实际条目为主」的迁移。
//!
//! 规则集中在纯函数 [`derive`]：读路径（status 等）即时推导，写路径
//! （mcp:apply、policy:set/config:import 涉及 MCP 时）在同一事务里落库并清掉旧数据。
//! 迁移只写数据库，从不改动任何助手配置文件。
//!
//! | 旧数据 | 结果 |
//! | --- | --- |
//! | 已分配，且该助手文件里有同名条目 | 以实际为准，丢弃分配记录 |
//! | 已分配，但文件里没有 / 分配已停用 | 该助手的停用快照（引用形态，env 写回时解析） |
//! | 定义从未分配，或库里定义比已应用的新，或只能落到不支持的助手 | 未分配的已保存定义 |

use anyhow::{bail, Result};
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

use super::inventory::TargetScan;
use super::model::{canonicalize_agent_id, digest_of, is_sha256_hex, is_valid_server_name};
use super::policy_view::{
    is_remote_definition, placeholder_name, validate_definition, validate_definition_env,
};
use super::saved::{self, neutral_definition, Resolve, SavedRow, ORIGIN_MIGRATED};
use super::targets::Format;
use crate::native::Context;
use crate::store;

/// 旧 `mcp_bindings` 的一行。
#[derive(Clone, Debug)]
pub struct LegacyBinding {
    pub version: String,
    pub definition: Value,
    pub enabled: bool,
    pub assigned: bool,
}

pub type LegacyBindings = BTreeMap<String, BTreeMap<String, LegacyBinding>>;

/// 旧版本号：去掉非定义字段后的规范摘要（与旧实现逐字节一致）。
fn definition_version(definition: &Value) -> String {
    let mut out = Map::new();
    if let Some(map) = definition.as_object() {
        for (key, value) in map {
            if key.starts_with('_') || key == "enabled" || key == "agents" {
                continue;
            }
            out.insert(key.clone(), value.clone());
        }
    }
    digest_of(&Value::Object(out))
}

fn validate_binding(server: &str, agent: &str, binding: &LegacyBinding) -> Result<()> {
    if !is_valid_server_name(server) {
        bail!("mcp_bindings 中存在非法 server 名: '{}'", server);
    }
    if canonicalize_agent_id(agent) != agent {
        bail!(
            "mcp_bindings 中存在非规范 Agent key '{}'（应为 '{}'），拒绝读取",
            agent,
            canonicalize_agent_id(agent)
        );
    }
    if !binding.definition.is_object() {
        bail!("mcp_bindings 中 definition 必须是 JSON 对象");
    }
    if binding.assigned {
        if !is_sha256_hex(&binding.version) {
            bail!("绑定条目 {}@{} 版本必须是 SHA-256 摘要", server, agent);
        }
        if binding.version != definition_version(&binding.definition) {
            bail!(
                "绑定条目 {}@{} 版本与定义摘要不一致，fail-closed 拒绝",
                server,
                agent
            );
        }
    }
    Ok(())
}

pub fn bindings_table_exists(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='mcp_bindings'",
        [],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

pub fn load_bindings_from(conn: &Connection) -> Result<LegacyBindings> {
    let mut out = LegacyBindings::new();
    if !bindings_table_exists(conn)? {
        return Ok(out);
    }
    let mut stmt = conn.prepare(
        "SELECT server, agent, version, definition_json, enabled, assigned FROM mcp_bindings",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
        ))
    })?;
    for row in rows {
        let (server, agent, version, definition_json, enabled, assigned) = row?;
        let definition: Value = serde_json::from_str(&definition_json)
            .map_err(|_| anyhow::anyhow!("mcp_bindings 中 {server}@{agent} 的定义 JSON 损坏"))?;
        let binding = LegacyBinding {
            version,
            definition,
            enabled: enabled != 0,
            assigned: assigned != 0,
        };
        validate_binding(&server, &agent, &binding)?;
        out.entry(server).or_default().insert(agent, binding);
    }
    Ok(out)
}

pub fn load_bindings_readonly(ctx: &Context) -> Result<LegacyBindings> {
    match store::open_readonly(ctx)? {
        Some(conn) => load_bindings_from(&conn),
        None => Ok(LegacyBindings::new()),
    }
}

/// policy 里仍在的旧服务库定义（去掉 `_delete` 遮蔽的）。
fn legacy_definitions(policy: &Value) -> BTreeMap<String, Value> {
    policy
        .get("mcp")
        .and_then(|mcp| mcp.get("definitions"))
        .and_then(Value::as_object)
        .map(|defs| {
            defs.iter()
                .filter(|(_, def)| {
                    def.is_object() && def.get("_delete").and_then(Value::as_bool) != Some(true)
                })
                .map(|(name, def)| (name.clone(), def.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// policy.mcp 除了空的 definitions 之外还有东西（定义、库级开关或删除遮蔽）。
fn policy_has_legacy(policy: &Value) -> bool {
    let Some(mcp) = policy.get("mcp").and_then(Value::as_object) else {
        return false;
    };
    mcp.iter().any(|(key, value)| {
        key != "definitions" || value.as_object().map_or(true, |defs| !defs.is_empty())
    })
}

/// 旧定义里的明文 env/header（旧版本只能经裸写入库产生）不能带进新表：改成
/// 同名变量引用，保留结构、丢弃明文。
fn sanitize_definition(def: &Value) -> Value {
    let mut out = neutral_definition(def);
    for field in ["env", "headers"] {
        if let Some(Value::Object(map)) = out.get_mut(field) {
            for (key, value) in map.iter_mut() {
                if placeholder_name(value).is_none() {
                    let mut var: String = key
                        .chars()
                        .map(|c| {
                            if c.is_ascii_alphanumeric() || c == '_' {
                                c.to_ascii_uppercase()
                            } else {
                                '_'
                            }
                        })
                        .collect();
                    if var.is_empty() || var.starts_with(|c: char| c.is_ascii_digit()) {
                        var.insert(0, '_');
                    }
                    *value = Value::String(format!("${{{var}}}"));
                }
            }
        }
    }
    out
}

/// 旧定义按目标客户端渲染成快照（引用形态；env 的 `${VAR}` 写回时解析）。
fn snapshot_entry(format: &Format, agent: &str, def: &Value) -> Result<(Value, Resolve)> {
    let server = "snapshot";
    validate_definition(def, server)?;
    validate_definition_env(def)?;
    let mut entry = Map::new();
    let mut resolve = Resolve::default();
    if is_remote_definition(def) {
        let url = def.get("url").and_then(Value::as_str).unwrap_or_default();
        let headers = def
            .get("headers")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let var_of = |value: &Value| placeholder_name(value).unwrap_or_default();
        match format {
            Format::ClaudeJson => {
                entry.insert("type".into(), json!("http"));
                entry.insert("url".into(), json!(url));
                if !headers.is_empty() {
                    entry.insert("headers".into(), Value::Object(headers));
                }
            }
            Format::CodexToml => {
                entry.insert("url".into(), json!(url));
                if !headers.is_empty() {
                    let refs: Map<String, Value> = headers
                        .iter()
                        .map(|(k, v)| (k.clone(), json!(var_of(v))))
                        .collect();
                    entry.insert("env_http_headers".into(), Value::Object(refs));
                }
            }
            Format::OpenCodeJson => {
                entry.insert("type".into(), json!("remote"));
                entry.insert("url".into(), json!(url));
                if !headers.is_empty() {
                    let refs: Map<String, Value> = headers
                        .iter()
                        .map(|(k, v)| (k.clone(), json!(format!("{{env:{}}}", var_of(v)))))
                        .collect();
                    entry.insert("headers".into(), Value::Object(refs));
                }
                entry.insert("oauth".into(), json!(false));
            }
            Format::Unsupported(fmt) => bail!("不支持的 MCP 配置格式: {fmt}"),
        }
        return Ok((Value::Object(entry), resolve));
    }
    let command = def
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut args: Vec<Value> = def
        .get("args")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(extra) = def
        .get("extra_args_by_agent")
        .and_then(|m| m.get(agent))
        .and_then(Value::as_array)
    {
        args.extend(extra.iter().cloned());
    }
    let env = def
        .get("env")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    resolve.env = env.keys().cloned().collect();
    match format {
        Format::ClaudeJson => {
            entry.insert("command".into(), json!(command));
            entry.insert("args".into(), Value::Array(args));
            entry.insert("env".into(), Value::Object(env));
        }
        Format::CodexToml => {
            entry.insert("command".into(), json!(command));
            entry.insert("args".into(), Value::Array(args));
            if let Some(timeout) = def.get("startup_timeout_sec") {
                entry.insert("startup_timeout_sec".into(), timeout.clone());
            }
            entry.insert("env".into(), Value::Object(env));
        }
        Format::OpenCodeJson => {
            let mut full = vec![json!(command)];
            full.extend(args);
            entry.insert("type".into(), json!("local"));
            entry.insert("command".into(), Value::Array(full));
            entry.insert("environment".into(), Value::Object(env));
        }
        Format::Unsupported(fmt) => bail!("不支持的 MCP 配置格式: {fmt}"),
    }
    Ok((Value::Object(entry), resolve))
}

/// 迁移规则（纯函数）。`scans` 用来判断各助手文件里是否已有同名条目。
pub fn derive(policy: &Value, bindings: &LegacyBindings, scans: &[TargetScan]) -> Vec<SavedRow> {
    let defs = legacy_definitions(policy);
    let mut rows = Vec::new();
    let mut represented: BTreeSet<String> = BTreeSet::new();
    let mut keep_definition: BTreeSet<String> = BTreeSet::new();

    for (server, per_agent) in bindings {
        for (agent, binding) in per_agent {
            if !binding.assigned {
                continue;
            }
            let scan = scans.iter().find(|s| s.target.agent == *agent);
            let present = scan.map_or(false, |s| s.error.is_none() && s.entry(server).is_some());
            let current_version = defs.get(server).map(definition_version);
            let up_to_date = current_version
                .as_deref()
                .map_or(true, |v| v == binding.version);
            if present {
                // 已分配已启用且库内定义没变：实际即当前，定义不用另存。
                if !(binding.enabled && up_to_date) {
                    keep_definition.insert(server.clone());
                }
                continue;
            }
            let def = defs.get(server).unwrap_or(&binding.definition);
            let snapshot = scan
                .filter(|s| !matches!(s.target.format, Format::Unsupported(_)))
                .map(|s| snapshot_entry(&s.target.format, agent, def));
            match snapshot {
                Some(Ok((entry, resolve))) => {
                    rows.push(SavedRow {
                        server: server.clone(),
                        agent: agent.clone(),
                        entry,
                        resolve,
                        origin: ORIGIN_MIGRATED.to_string(),
                    });
                    represented.insert(server.clone());
                }
                _ => {
                    keep_definition.insert(server.clone());
                    if !defs.contains_key(server) {
                        // 孤儿分配且写不成快照：按其自带定义另存，避免丢失。
                        rows.push(unassigned_row(server, &binding.definition));
                        represented.insert(server.clone());
                    }
                }
            }
        }
    }

    for (name, def) in &defs {
        if represented.contains(name) {
            continue;
        }
        let assigned = bindings
            .get(name)
            .map_or(false, |per_agent| per_agent.values().any(|b| b.assigned));
        if !assigned || keep_definition.contains(name) {
            rows.push(unassigned_row(name, def));
        }
    }
    rows.retain(|row| is_valid_server_name(&row.server));
    rows.sort_by(|a, b| (&a.server, &a.agent).cmp(&(&b.server, &b.agent)));
    rows.dedup_by(|a, b| a.server == b.server && a.agent == b.agent);
    rows
}

fn unassigned_row(name: &str, def: &Value) -> SavedRow {
    SavedRow {
        server: name.to_string(),
        agent: String::new(),
        entry: sanitize_definition(def),
        resolve: Resolve::default(),
        origin: ORIGIN_MIGRATED.to_string(),
    }
}

/// 写路径迁移：在调用方的 Immediate 事务里把旧数据落进 `mcp_saved` 并清掉旧数据。
/// 没有旧数据时什么都不写。返回是否发生了迁移。
pub fn migrate(conn: &Connection, scans: &[TargetScan], run_id: Option<&str>) -> Result<bool> {
    let mut policy = store::policy_from_connection(conn)?;
    let has_table = bindings_table_exists(conn)?;
    if !has_table && !policy_has_legacy(&policy) {
        return Ok(false);
    }
    let bindings = load_bindings_from(conn)?;
    let derived = derive(&policy, &bindings, scans);
    saved::ensure_table(conn)?;
    let mut snapshots = 0usize;
    let mut unassigned = 0usize;
    for row in &derived {
        if !row.agent.is_empty() {
            if saved::exists(conn, &row.server, &row.agent)? {
                continue;
            }
            snapshots += 1;
        } else {
            unassigned += 1;
        }
        saved::upsert(conn, row)?;
    }
    // 旧定义里带 `_delete` 遮蔽的：同名未分配已保存定义一并删除（旧语义就是删定义）。
    if let Some(defs) = policy
        .get("mcp")
        .and_then(|m| m.get("definitions"))
        .and_then(Value::as_object)
    {
        for (name, def) in defs {
            if def.get("_delete").and_then(Value::as_bool) == Some(true) {
                saved::delete(conn, name, "")?;
            }
        }
    }
    if has_table {
        conn.execute_batch("DROP TABLE IF EXISTS mcp_bindings")?;
    }
    if let Some(map) = policy.as_object_mut() {
        map.insert("mcp".into(), json!({"definitions": {}}));
    }
    store::write_policy(conn, &policy)?;
    store::record_operation(
        conn,
        run_id,
        "mcp:migrate",
        None,
        Some(&json!({"snapshots": snapshots, "saved": unassigned})),
    )?;
    Ok(true)
}
