use super::{recipe, render, text};
use crate::native_aliases::model;
use crate::native_mcp::model::sha256_hex;
use crate::native_mcp::rollback::{atomic_write_bytes_checked, RollbackGuard};
use crate::{Context, Request};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const WINDOWS_MARK: &str = "# Agent Manager launch shortcut\n";

struct Snapshot {
    path: PathBuf,
    files: BTreeMap<PathBuf, Option<Vec<u8>>>,
    base: String,
    entries: Vec<Value>,
    external: Vec<String>,
}

pub(super) struct Change {
    path: PathBuf,
    old: Option<Vec<u8>>,
    new: Option<Vec<u8>>,
}

fn read(ctx: &Context, path: &Path) -> Result<(PathBuf, Option<Vec<u8>>)> {
    crate::native_profiles::paths::validate_shell_target(ctx, path, false)?;
    let real = if path.exists() {
        std::fs::canonicalize(path)?
    } else {
        path.to_path_buf()
    };
    crate::native_mcp::targets::validate_target_path(&real)?;
    match std::fs::read(&real) {
        Ok(bytes) => {
            std::str::from_utf8(&bytes)?;
            Ok((real, Some(bytes)))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((real, None)),
        Err(error) => Err(error.into()),
    }
}

fn shell(req: &Request) -> Result<String> {
    let value = req
        .value("--shell")
        .unwrap_or(if cfg!(windows) { "powershell" } else { "zsh" });
    if (cfg!(windows) && value != "powershell")
        || (!cfg!(windows) && !matches!(value, "zsh" | "bash"))
    {
        bail!("This shell cannot be applied on the current platform");
    }
    Ok(value.into())
}

fn snapshot(ctx: &Context, shell: &str) -> Result<Snapshot> {
    if !ctx.home_dir.is_absolute() {
        bail!("HOME must be absolute");
    }
    if cfg!(windows) {
        return windows_snapshot(ctx);
    }
    let dir = if shell == "zsh" {
        std::env::var_os("ZDOTDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| ctx.home_dir.clone())
    } else {
        ctx.home_dir.clone()
    };
    if !dir.is_absolute() {
        bail!("ZDOTDIR must be absolute");
    }
    let path = dir.join(if shell == "zsh" { ".zshrc" } else { ".bashrc" });
    let (path, bytes) = read(ctx, &path)?;
    let split = model::split_block(std::str::from_utf8(bytes.as_deref().unwrap_or_default())?)?;
    let entries = split
        .entries()
        .iter()
        .map(|entry| {
            let command = entry.line.split_once('=').and_then(|(_, rhs)| {
                let inner = rhs.strip_prefix('\'')?.strip_suffix('\'')?;
                let value = inner.replace("'\\''", "'");
                (render::posix(&value) == rhs).then_some(value)
            });
            json!({"name":entry.name,"command":command,"definition":entry.line,"path":path})
        })
        .collect();
    let external = split
        .external()
        .iter()
        .map(|entry| entry.name.clone())
        .collect();
    let base = bytes
        .as_deref()
        .map(|value| sha256_hex(std::str::from_utf8(value).expect("validated UTF-8")))
        .unwrap_or_else(|| "missing".into());
    Ok(Snapshot {
        path: path.clone(),
        files: BTreeMap::from([(path, bytes)]),
        base,
        entries,
        external,
    })
}

fn windows_snapshot(ctx: &Context) -> Result<Snapshot> {
    let path = ctx.home_dir.join(".agent-manager/launchers");
    crate::native_profiles::paths::validate_shell_target(ctx, &path, true)?;
    let mut files = BTreeMap::new();
    let mut entries = Vec::new();
    let mut external = Vec::new();
    if path.exists() {
        for entry in std::fs::read_dir(&path)? {
            let file = entry?.path();
            if !file
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("ps1"))
            {
                continue;
            }
            let (real, bytes) = read(ctx, &file)?;
            let content = std::str::from_utf8(bytes.as_deref().unwrap_or_default())?;
            let name = file
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if let Some(command) = content.strip_prefix(WINDOWS_MARK) {
                entries.push(json!({"name":name,"command":command.trim_end(),"definition":content,"path":file}));
            } else {
                external.push(name);
            }
            files.insert(real, bytes);
        }
    }
    entries.sort_by_key(|v| v["name"].as_str().unwrap_or_default().to_owned());
    let base = if files.is_empty() {
        "missing".into()
    } else {
        sha256_hex(&serde_json::to_string(&files)?)
    };
    Ok(Snapshot {
        path,
        files,
        base,
        entries,
        external,
    })
}

fn definitions(req: &Request) -> Result<Vec<(String, Option<String>)>> {
    if let Some(name) = req.value("--remove") {
        if req.value("--spec").is_some() || !model::valid_alias_name(name) {
            bail!("Invalid shortcut removal");
        }
        return Ok(vec![(name.into(), None)]);
    }
    let spec = recipe(req)?;
    if spec.platform != std::env::consts::OS {
        bail!("Only the current platform can be applied locally");
    }
    let expected_shell = if spec.platform == "windows" {
        "powershell"
    } else {
        spec.shell.as_str()
    };
    if req
        .value("--shell")
        .is_some_and(|value| value != expected_shell)
    {
        bail!("Shell does not match the recipe");
    }
    Ok(spec
        .variants
        .iter()
        .map(|v| {
            let entry = render::build(&spec, v);
            (entry.name, Some(entry.definition))
        })
        .collect())
}

fn plan(ctx: &Context, req: &Request) -> Result<(Snapshot, Vec<Change>)> {
    let definitions = definitions(req)?;
    let selected_shell = selected_shell(req)?;
    let state = snapshot(ctx, &selected_shell)?;
    if req.value("--base") != Some(state.base.as_str()) {
        bail!("Modified externally; refresh before applying");
    }
    for (name, _) in &definitions {
        if state
            .external
            .iter()
            .any(|other| other.eq_ignore_ascii_case(name))
        {
            bail!("Shortcut name conflicts with an unmanaged entry");
        }
    }
    let mut changes = Vec::new();
    if cfg!(windows) {
        for (name, definition) in definitions {
            let path = state.path.join(format!("{name}.ps1"));
            let (path, old) = read(ctx, &path)?;
            if old
                .as_deref()
                .is_some_and(|value| !value.starts_with(WINDOWS_MARK.as_bytes()))
            {
                bail!("Shortcut name conflicts with an unmanaged file");
            }
            let new = definition.map(String::into_bytes);
            if old != new {
                changes.push(Change { path, old, new });
            }
        }
    } else {
        let old = state.files.get(&state.path).cloned().flatten();
        let split = model::split_block(std::str::from_utf8(old.as_deref().unwrap_or_default())?)?;
        let mut lines = split.block.unwrap_or_default();
        for (name, definition) in definitions {
            lines.retain(|line| model::parse_alias_line(line).map_or(true, |v| v.name != name));
            if let Some(line) = definition {
                lines.push(line);
            }
        }
        if let Some(new) = model::converge_content(
            old.as_deref().unwrap_or_default(),
            &model::block_inner(&lines),
        )? {
            changes.push(Change {
                path: state.path.clone(),
                old,
                new: Some(new),
            });
        }
    }
    Ok((state, changes))
}

fn changes_json(changes: &[Change]) -> Value {
    json!(changes.iter().map(|c|json!({"path":c.path,
        "original":c.old.as_deref().map(|v|String::from_utf8_lossy(v).into_owned()),
        "content":c.new.as_deref().map(|v|String::from_utf8_lossy(v).into_owned()).unwrap_or_default(),
        "deleted":c.new.is_none()})).collect::<Vec<_>>())
}

fn apply(ctx: &Context, req: &Request) -> Result<Value> {
    if !req.flag("--confirm") {
        bail!("Explicit confirmation is required");
    }
    plan(ctx, req)?; // Invalid or stale requests must not create a lock or database.
    let _lock = crate::native::acquire_apply_lock(ctx, "Launch shortcuts")?;
    let (state, changes) = plan(ctx, req)?;
    let mut conn = crate::store::open_write(ctx)?;
    let tx = conn.transaction()?;
    let mut guard = RollbackGuard::new();
    let mut deleted = Vec::new();
    let run = (|| -> Result<()> {
        for change in &changes {
            let (resolved, _) = read(ctx, &change.path)?;
            if resolved != change.path {
                bail!("Shortcut path changed externally");
            }
            if let Some(new) = &change.new {
                atomic_write_bytes_checked(&change.path, new, change.old.as_deref())?;
                guard.record_written(&change.path, change.old.clone(), new.clone());
            } else if let Some(old) = &change.old {
                if std::fs::read(&change.path)? != *old {
                    bail!("Modified externally");
                }
                // Record the owned bytes before unlinking so failures restore only an absent file.
                std::fs::remove_file(&change.path)?;
                deleted.push(change);
            }
        }
        crate::store::record_operation(
            &tx,
            Some(&req.run_id),
            "launch:apply",
            Some("shortcuts"),
            Some(
                &json!({"names":definitions(req)?.iter().map(|v|&v.0).collect::<Vec<_>>(),"before":state.base}),
            ),
        )?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(error) = run {
        let restored = guard.rollback();
        for change in deleted {
            if !change.path.exists() {
                if let Some(old) = &change.old {
                    atomic_write_bytes_checked(&change.path, old, None)?;
                }
            }
        }
        restored?;
        return Err(error);
    }
    guard.commit();
    Ok(json!({"files":changes_json(&changes),"status":status(ctx,&selected_shell(req)?)?}))
}

fn selected_shell(req: &Request) -> Result<String> {
    match req.value("--spec") {
        Some(_) => Ok(recipe(req)?.shell),
        None => shell(req),
    }
}

fn status(ctx: &Context, shell: &str) -> Result<Value> {
    let state = snapshot(ctx, shell)?;
    Ok(
        json!({"platform":std::env::consts::OS,"shell":shell,"path":state.path,
        "base":state.base,"entries":state.entries,"external_names":state.external}),
    )
}

pub(super) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let command = req.command.replace('.', ":");
    let allowed = match command.as_str() {
        "launch:status" => vec!["--shell"],
        "launch:preview" => vec!["--shell", "--spec", "--remove", "--base"],
        "launch:apply" => vec!["--shell", "--spec", "--remove", "--base", "--confirm"],
        _ => bail!("Unknown launch operation"),
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut iter = req.args.iter();
    while let Some(flag) = iter.next() {
        if !allowed.contains(&flag.as_str()) || !seen.insert(flag) {
            bail!("Unknown or repeated argument");
        }
        if flag != "--confirm" {
            text(
                iter.next()
                    .ok_or_else(|| anyhow::anyhow!("Missing argument value"))?,
                65536,
            )?;
        }
    }
    match command.as_str() {
        "launch:status" => status(ctx, &shell(req)?),
        "launch:preview" => {
            let (state, changes) = plan(ctx, req)?;
            Ok(json!({"base":state.base,"files":changes_json(&changes)}))
        }
        "launch:apply" => apply(ctx, req),
        _ => unreachable!(),
    }
}
