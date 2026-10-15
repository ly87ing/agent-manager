use super::Target;
use crate::native_mcp::model::sha256_hex;
use crate::{store, Context, Request};
use anyhow::{bail, Result};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS environment_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    profile_json TEXT NOT NULL
)";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    name: String,
    target: Target,
    #[serde(default)]
    auto_sync: bool,
}

impl Profile {
    fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty()
            || self.name.len() > 120
            || self.name.chars().any(char::is_control)
        {
            bail!("Enter a collection name of at most 120 bytes without control characters.");
        }
        self.target.validate()?;
        let target = &self.target;
        if target.timezone.is_none()
            && target.languages.is_empty()
            && target.region.is_none()
            && target.locale.is_none()
            && target.ntp_server.is_none()
            && target.automatic_time.is_none()
            && target.time_synchronized.is_none()
            && target.automatic_timezone.is_none()
            && target.max_clock_offset_ms.is_none()
        {
            bail!("Choose at least one expected value.");
        }
        if self.auto_sync {
            target.validate_time_repair()?;
        }
        Ok(())
    }
}

fn has_table(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='environment_profiles'",
        [],
        |row| row.get::<_, i64>(0),
    )? > 0)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BackupItem {
    id: String,
    profile: Profile,
}

pub(crate) fn backup(ctx: &Context) -> Result<Vec<Value>> {
    let Some(conn) = store::open_readonly(ctx)? else {
        return Ok(vec![]);
    };
    if !has_table(&conn)? {
        return Ok(vec![]);
    }
    let mut query =
        conn.prepare("SELECT id, profile_json FROM environment_profiles ORDER BY id")?;
    let rows = query.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut items = Vec::new();
    for row in rows {
        let (id, raw) = row?;
        let profile: Profile = serde_json::from_str(&raw)
            .map_err(|_| anyhow::anyhow!("Invalid saved expected-value collection."))?;
        profile.validate()?;
        let value = serde_json::to_value(BackupItem { id, profile })?;
        crate::native::reject_secrets_value(&value)?;
        items.push(value);
    }
    Ok(items)
}

pub(crate) fn validate_backup(value: &Value) -> Result<Vec<(String, String)>> {
    let items = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("environment_profiles must be an array."))?;
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut rows = Vec::new();
    for value in items {
        let item: BackupItem = serde_json::from_value(value.clone())
            .map_err(|_| anyhow::anyhow!("Invalid environment backup item."))?;
        if item.id.is_empty()
            || item.id.len() > 64
            || !item
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || !ids.insert(item.id.clone())
        {
            bail!("Invalid or duplicate environment backup id.");
        }
        item.profile.validate()?;
        if !names.insert(item.profile.name.to_lowercase()) {
            bail!("Duplicate expected-value collection name.");
        }
        crate::native::reject_secrets_value(value)?;
        rows.push((item.id, serde_json::to_string(&item.profile)?));
    }
    Ok(rows)
}

pub(crate) fn restore_backup(conn: &Connection, rows: &[(String, String)]) -> Result<()> {
    conn.execute_batch(TABLE_DDL)?;
    conn.execute("DELETE FROM environment_profiles", [])?;
    for (id, raw) in rows {
        conn.execute(
            "INSERT INTO environment_profiles (id, profile_json) VALUES (?1, ?2)",
            rusqlite::params![id, raw],
        )?;
    }
    Ok(())
}

fn item(id: &str, raw: &str) -> Result<Value> {
    let profile: Profile = serde_json::from_str(raw)
        .map_err(|_| anyhow::anyhow!("Invalid saved expected-value collection."))?;
    profile.validate()?;
    Ok(
        json!({"id":id,"revision":sha256_hex(raw),"name":profile.name,"target":profile.target,"auto_sync":profile.auto_sync}),
    )
}

fn builtins() -> Vec<Value> {
    [
        ("builtin:los-angeles", "Los Angeles (US)", "America/Los_Angeles", "en-US", "US", "en_US.UTF-8", "time.nist.gov", "NIST", "https://tf.nist.gov/tf-cgi/servers.cgi"),
        ("builtin:singapore", "Singapore (SG)", "Asia/Singapore", "en-SG", "SG", "en_US.UTF-8", "nets.org.sg", "A*STAR NMC", "https://www.a-star.edu.sg/nmc/Research-and-Development/Labs/FL"),
    ].into_iter().map(|(id, name, timezone, language, region, locale, server, time_service, source_url)| {
        let target = Target {
            timezone: Some(timezone.into()), languages: vec![language.into()],
            region: Some(region.into()), locale: Some(locale.into()), strict_locale: true,
            ntp_server: Some(server.into()), automatic_time: Some(true),
            time_synchronized: Some(true), automatic_timezone: Some(false), max_clock_offset_ms: Some(2000),
        };
        json!({"id":id,"name":name,"target":target,"time_service":time_service,"source_url":source_url})
    }).collect()
}

fn list(conn: Option<&Connection>) -> Result<Vec<Value>> {
    let Some(conn) = conn else {
        return Ok(vec![]);
    };
    if !has_table(conn)? {
        return Ok(vec![]);
    }
    let mut statement =
        conn.prepare("SELECT id, profile_json FROM environment_profiles ORDER BY id")?;
    let mut items = Vec::new();
    for row in statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (id, raw) = row?;
        items.push(item(&id, &raw)?);
    }
    Ok(items)
}

fn check_base(conn: &Connection, id: &str, base: &str) -> Result<()> {
    let raw: Option<String> = if has_table(conn)? {
        conn.query_row(
            "SELECT profile_json FROM environment_profiles WHERE id=?1",
            [id],
            |row| row.get(0),
        )
        .optional()?
    } else {
        None
    };
    if raw.as_deref().map(sha256_hex).as_deref() != Some(base) {
        bail!("Expected-value collection changed or was removed; reload before editing.");
    }
    Ok(())
}

pub(super) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let command = req.command.replace('.', ":");
    if command == "environment:profiles" {
        if !req.args.is_empty() {
            bail!("profiles accepts no arguments.");
        }
        return Ok(
            json!({"items":list(store::open_readonly(ctx)?.as_ref())?,"builtins":builtins()}),
        );
    }
    let mut seen = BTreeSet::new();
    if req.args.len() % 2 != 0 {
        bail!("Collection flags require values.");
    }
    for pair in req.args.chunks_exact(2) {
        if !(matches!(pair[0].as_str(), "--id" | "--base")
            || (command == "environment:save-profile" && pair[0] == "--spec"))
            || !seen.insert(&pair[0])
        {
            bail!("Unsupported or duplicate collection flag.");
        }
    }
    let id = req.value("--id");
    let base = req.value("--base");
    if id.is_some() != base.is_some() || (command == "environment:delete-profile" && id.is_none()) {
        bail!("Editing a collection requires --id and --base.");
    }
    if id.is_some_and(|v| {
        v.is_empty()
            || v.len() > 64
            || !v
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    }) || base.is_some_and(|v| {
        v.len() != 64
            || !v
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        bail!("Invalid collection id or revision.");
    }
    let profile = if command == "environment:save-profile" {
        let raw = req
            .value("--spec")
            .filter(|v| v.len() <= 8192)
            .ok_or_else(|| anyhow::anyhow!("Missing or oversized collection."))?;
        let profile: Profile =
            serde_json::from_str(raw).map_err(|_| anyhow::anyhow!("Invalid collection fields."))?;
        profile.validate()?;
        crate::native::reject_secrets_value(&serde_json::to_value(&profile)?)?;
        Some(profile)
    } else {
        None
    };
    if let Some(id) = id {
        let conn = store::open_readonly(ctx)?
            .ok_or_else(|| anyhow::anyhow!("Collection no longer exists."))?;
        check_base(&conn, id, base.expect("validated base"))?;
    }
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(id) = id {
        check_base(&tx, id, base.expect("validated base"))?;
    }
    let id = id.map(str::to_string).unwrap_or_else(store::uuid_v4);
    let result = if let Some(profile) = profile {
        if list(Some(&tx))?.iter().any(|saved| {
            saved["id"] != id
                && saved["name"]
                    .as_str()
                    .is_some_and(|name| name.to_lowercase() == profile.name.to_lowercase())
        }) {
            bail!("A collection with this name already exists.");
        }
        let raw = serde_json::to_string(&profile)?;
        tx.execute_batch(TABLE_DDL)?;
        tx.execute("INSERT INTO environment_profiles (id, profile_json) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET profile_json=excluded.profile_json", rusqlite::params![id,raw])?;
        json!({"item":item(&id, &raw)?})
    } else {
        tx.execute("DELETE FROM environment_profiles WHERE id=?1", [&id])?;
        json!({"id":id})
    };
    store::record_operation(&tx, Some(&req.run_id), &command, Some(&id), None)?;
    tx.commit()?;
    Ok(result)
}
