use super::{recipe, render, text, Recipe};
use crate::native_mcp::model::sha256_hex;
use crate::{store, Context, Request};
use anyhow::{bail, Result};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS launch_recipes (
    id TEXT PRIMARY KEY NOT NULL,
    recipe_json TEXT NOT NULL
)";

struct Saved {
    id: String,
    raw: String,
    recipe: Recipe,
}

impl Saved {
    fn parse(id: String, raw: String) -> Result<Self> {
        let recipe: Recipe = serde_json::from_str(&raw)
            .map_err(|_| anyhow::anyhow!("Invalid saved launch configuration"))?;
        recipe.validate()?;
        Ok(Self { id, raw, recipe })
    }

    fn revision(&self) -> String {
        sha256_hex(&self.raw)
    }

    fn json(&self) -> Value {
        let entries = self
            .recipe
            .variants
            .iter()
            .map(|variant| render::build(&self.recipe, variant))
            .collect::<Vec<_>>();
        json!({"id":self.id,"revision":self.revision(),"recipe":self.recipe,"entries":entries})
    }
}

fn has_table(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='launch_recipes'",
        [],
        |row| row.get::<_, i64>(0),
    )? > 0)
}

fn list(ctx: &Context) -> Result<Value> {
    let Some(conn) = store::open_readonly(ctx)? else {
        return Ok(json!({"items":[]}));
    };
    if !has_table(&conn)? {
        return Ok(json!({"items":[]}));
    }
    let mut query = conn.prepare("SELECT id, recipe_json FROM launch_recipes ORDER BY id")?;
    let rows = query.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut items = Vec::new();
    for row in rows {
        let (id, raw) = row?;
        items.push(Saved::parse(id, raw)?.json());
    }
    Ok(json!({"items":items}))
}

fn existing(conn: &Connection, id: &str, base: &str) -> Result<Saved> {
    let raw: Option<String> = if has_table(conn)? {
        conn.query_row(
            "SELECT recipe_json FROM launch_recipes WHERE id=?1",
            [id],
            |row| row.get(0),
        )
        .optional()?
    } else {
        None
    };
    let item = Saved::parse(
        id.into(),
        raw.ok_or_else(|| {
            anyhow::anyhow!("Saved configuration no longer exists; reload before editing")
        })?,
    )?;
    if item.revision() != base {
        bail!("Saved configuration changed; reload before editing");
    }
    Ok(item)
}

fn validate_args(req: &Request, command: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    if req.args.len() % 2 != 0 {
        bail!("Launch configuration flags require values");
    }
    for pair in req.args.chunks_exact(2) {
        if !matches!(pair[0].as_str(), "--id" | "--base")
            && !(command == "launch:save" && pair[0] == "--spec")
            || !seen.insert(&pair[0])
        {
            bail!("Unsupported or duplicate launch configuration flag");
        }
    }
    if let Some(id) = req.value("--id") {
        text(id, 64)?;
        if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            bail!("Invalid launch configuration id");
        }
    }
    if req.value("--id").is_some() != req.value("--base").is_some()
        || (command == "launch:delete" && req.value("--id").is_none())
    {
        bail!("Editing a saved configuration requires --id and --base");
    }
    Ok(())
}

pub(super) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let command = req.command.replace('.', ":");
    if command == "launch:list" {
        if !req.args.is_empty() {
            bail!("list accepts no arguments");
        }
        return list(ctx);
    }
    validate_args(req, &command)?;
    let spec = if command == "launch:save" {
        let spec = recipe(req)?;
        crate::native::reject_secrets_value(&serde_json::to_value(&spec)?)?;
        Some(spec)
    } else {
        None
    };
    if let Some(id) = req.value("--id") {
        let conn = store::open_readonly(ctx)?
            .ok_or_else(|| anyhow::anyhow!("Saved configuration no longer exists"))?;
        existing(&conn, id, req.value("--base").expect("validated base"))?;
    }
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let id = req
        .value("--id")
        .map(str::to_owned)
        .unwrap_or_else(store::uuid_v4);
    if req.value("--id").is_some() {
        existing(&tx, &id, req.value("--base").expect("validated base"))?;
    }
    let result = if let Some(spec) = spec {
        let raw = serde_json::to_string(&spec)?;
        tx.execute_batch(TABLE_DDL)?;
        tx.execute(
            "INSERT INTO launch_recipes (id, recipe_json) VALUES (?1, ?2)
            ON CONFLICT(id) DO UPDATE SET recipe_json=excluded.recipe_json",
            rusqlite::params![id, raw],
        )?;
        json!({"item":Saved::parse(id.clone(), raw)?.json()})
    } else {
        tx.execute("DELETE FROM launch_recipes WHERE id=?1", [&id])?;
        json!({"id":id})
    };
    store::record_operation(&tx, Some(&req.run_id), &command, Some(&id), None)?;
    tx.commit()?;
    Ok(result)
}
