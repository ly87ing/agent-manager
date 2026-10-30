//! One selected connection field per confirmed operation, isolated from full cleanup.
use super::{
    connection_inspect as inspection, connection_json, connection_session, connection_shell,
    connection_sources,
};
use crate::{Context, Request};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(super) struct Plan {
    pub path: std::path::PathBuf,
    pub filename: String,
    pub key: String,
    pub item: String,
    pub bytes: Vec<u8>,
    pub output: Vec<u8>,
    pub base: String,
    pub parent_identity: Vec<u8>,
    pub blockers: Vec<String>,
    pub kind: &'static str,
    pub line: Option<usize>,
    pub root: Option<String>,
}
struct Selection {
    item: String,
    filename: String,
    key: String,
    kind: &'static str,
    line: Option<usize>,
}

fn selection(value: &str) -> Result<Selection> {
    let items: Vec<String> = serde_json::from_str(value)
        .map_err(|_| anyhow::anyhow!("Choose exactly one listed connection field."))?;
    if items.len() != 1 {
        bail!("Choose exactly one listed connection field per confirmation.");
    }
    let parts: Vec<_> = items[0].split(':').collect();
    let (filename, key, kind, line) = match parts.as_slice() {
        [id, key]
            if inspection::FILES.iter().any(|(file, _)| file == id) && inspection::is_key(key) =>
        {
            (
                inspection::FILES
                    .iter()
                    .find(|(file, _)| file == id)
                    .unwrap()
                    .1
                    .to_string(),
                key.to_string(),
                "settings",
                None,
            )
        }
        [id, "root", key]
            if inspection::FILES.iter().any(|(file, _)| file == id)
                && matches!(*key, "apiKeyHelper" | "PROXY_MANAGED") =>
        {
            (
                inspection::FILES
                    .iter()
                    .find(|(file, _)| file == id)
                    .unwrap()
                    .1
                    .to_string(),
                key.to_string(),
                "settings_root",
                None,
            )
        }
        ["shell", file, line, key]
            if connection_shell::FILES.contains(file) && inspection::shared_key(key) =>
        {
            let line = line
                .parse::<usize>()
                .ok()
                .filter(|line| *line > 0 && *line <= 1048576)
                .ok_or_else(|| anyhow::anyhow!("Invalid startup line."))?;
            (file.to_string(), key.to_string(), "startup", Some(line))
        }
        ["user-env", key] if inspection::shared_key(key) => {
            (key.to_string(), key.to_string(), "user_environment", None)
        }
        _ => bail!("Unknown or protected connection field."),
    };
    Ok(Selection {
        item: items[0].clone(),
        filename,
        key,
        kind,
        line,
    })
}

fn writers() -> Vec<String> {
    let mut blockers = Vec::new();
    match super::processes::active() {
        Ok(processes) if processes.is_empty() => {}
        Ok(_) => blockers.push(
            "Close Claude applications and sessions yourself, then request a new preview.".into(),
        ),
        Err(_) => blockers.push("Claude process inspection is unavailable.".into()),
    }
    if connection_sources::cc_switch() != "not_running" {
        blockers.push(
            "CC Switch is running or its state is unknown; close it and request a new preview."
                .into(),
        );
    }
    blockers
}

impl Plan {
    fn capture(ctx: &Context, selected: &str, root: Option<&str>) -> Result<Self> {
        let selected = selection(selected)?;
        let mut digest = Sha256::new();
        digest.update(selected.item.as_bytes());
        let mut blockers = writers();
        let (path, bytes, output, parent_identity) = if selected.kind == "user_environment" {
            let scope = connection_session::scope()?;
            let bytes = connection_session::read(&selected.key)?.ok_or_else(|| {
                anyhow::anyhow!("The selected user environment variable is absent.")
            })?;
            digest.update(scope.as_bytes());
            digest.update(&bytes);
            (std::path::PathBuf::from(scope), bytes, vec![], vec![])
        } else {
            let parent_path = if selected.kind == "startup" {
                inspection::home(ctx)?
            } else {
                inspection::settings_root(ctx, root)?
            };
            let path = parent_path.join(&selected.filename);
            let bytes = inspection::read(&path)?.ok_or_else(|| {
                anyhow::anyhow!("The selected file is absent; request a new inspection.")
            })?;
            let output = match selected.kind {
                "startup" => {
                    connection_shell::remove(&bytes, selected.line.unwrap(), &selected.key)?
                }
                "settings_root" => connection_json::remove_root(&bytes, &selected.key)?,
                _ => connection_json::remove(&bytes, &selected.key)?,
            };
            let dependencies = super::dependencies::caller_evidence(&[(path.clone(), false)])?;
            let parent =
                cap_std::fs::Dir::open_ambient_dir(&parent_path, cap_std::ambient_authority())?;
            if selected.kind != "startup" && super::retention::project(&parent)? {
                bail!("Project and worktree settings are protected.");
            }
            let entry = super::tree::entry(
                &parent,
                std::path::Path::new(&selected.filename),
                selected.filename.clone().into(),
            )?;
            if parent.read(&selected.filename)? != bytes {
                bail!("The selected file changed during inspection.");
            }
            let parent_identity = super::tree::identity(&parent)?;
            digest.update(path.as_os_str().as_encoded_bytes());
            digest.update(&parent_identity);
            digest.update(entry.hash);
            digest.update(dependencies);
            #[cfg(target_os = "macos")]
            if super::connection_publish::supported(&path).is_err() {
                blockers.push(
                    "The selected file's ownership or metadata could not be safely verified."
                        .into(),
                );
            }
            (path, bytes, output, parent_identity)
        };
        if !cfg!(target_os = "macos") {
            blockers.push("Targeted writes are currently available on macOS only; this platform supports inspection.".into());
        }
        Ok(Self {
            path,
            filename: selected.filename,
            key: selected.key,
            item: selected.item,
            bytes,
            output,
            base: format!("{:x}", digest.finalize()),
            parent_identity,
            blockers,
            kind: selected.kind,
            line: selected.line,
            root: root.map(str::to_string),
        })
    }
    fn data(&self) -> Value {
        json!({"scope":"claude_connections","base":self.base,"item":self.item,"path":self.path,"key":self.key,"kind":self.kind,"line":self.line,"root":self.root,"shared":matches!(self.kind,"startup"|"user_environment"),"can_clear":self.blockers.is_empty(),"blockers":self.blockers})
    }
}

pub(super) fn execute(ctx: &Context, request: &Request) -> Result<Value> {
    let clear = request.command.replace('.', ":") == "claude-data:clear";
    let mut selected = None;
    let mut base = None;
    let mut confirm = false;
    let mut root = None;
    let mut seen = std::collections::BTreeSet::new();
    let mut args = request.args.iter();
    while let Some(arg) = args.next() {
        if !seen.insert(arg.as_str()) {
            bail!("Duplicate connection-maintenance argument.");
        }
        match arg.as_str() {
            "--connection-items" => selected = args.next(),
            "--connection-root" => {
                root = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("Missing selected directory."))?
                        .as_str(),
                )
            }
            "--base" if clear => base = args.next(),
            "--confirm" if clear => confirm = true,
            "--offline" | "--json" => {}
            _ => bail!("Unsupported connection-maintenance argument."),
        }
    }
    let selected = selected.ok_or_else(|| anyhow::anyhow!("Missing selected connection field."))?;
    if clear
        && (!confirm
            || !base.is_some_and(|base| {
                base.len() == 64
                    && base
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }))
    {
        bail!("Targeted cleanup requires a fresh --base and explicit --confirm.");
    }
    let plan = Plan::capture(ctx, selected, root)?;
    if !clear {
        return Ok(plan.data());
    }
    if Some(&plan.base) != base || !plan.blockers.is_empty() {
        bail!(
            "The preview changed or is blocked; no settings were changed. Request a new preview."
        );
    }
    let _lock = crate::native::acquire_apply_lock(ctx, "Claude connection maintenance")?;
    let conn = crate::store::open_write(ctx)?;
    let audit = |status| {
        crate::store::record_operation(
            &conn,
            Some(&request.run_id),
            "claude-data:connection-clear",
            Some("selected_claude_connection"),
            Some(&json!({"status":status,"item":plan.item})),
        )
    };
    audit("requested")?;
    let guard = || -> Result<()> {
        let fresh = Plan::capture(ctx, selected, root)?;
        if fresh.base != plan.base || !fresh.blockers.is_empty() {
            bail!("The selected source or its writers changed; cleanup stopped.");
        }
        Ok(())
    };
    let applied: Result<()> = if plan.kind == "user_environment" {
        guard().and_then(|_| {
            connection_session::remove(&plan.key, plan.path.to_str().unwrap(), &plan.bytes)
        })
    } else {
        #[cfg(target_os = "macos")]
        {
            super::connection_publish::publish(&plan, guard)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = guard;
            Err(anyhow::anyhow!(
                "Targeted writes are unavailable on this platform."
            ))
        }
    };
    let retained = if plan.kind == "user_environment" {
        connection_session::scope()
            .is_ok_and(|scope| scope.as_bytes() == plan.path.as_os_str().as_encoded_bytes())
    } else {
        inspection::read(&plan.path).ok().flatten().as_deref() == Some(plan.output.as_slice())
    };
    let removed = if plan.kind == "user_environment" {
        connection_session::read(&plan.key).is_ok_and(|value| value.is_none())
    } else {
        retained
    };
    let verified = applied.is_ok() && retained && removed && writers().is_empty();
    let audit_ok = audit(if verified { "complete" } else { "partial" }).is_ok();
    let status = if verified && audit_ok {
        "complete"
    } else {
        "partial"
    };
    let message = if !audit_ok {
        "The completion audit is unavailable; inspect the selected source before continuing.".into()
    } else {
        applied.err().map(|error|error.to_string()).unwrap_or_else(||if verified {"Only the selected item was removed and checked. Other sources and existing process environments remain unchanged; verify future consumers separately.".into()}else{"Independent verification was incomplete. Inspect the selected source before another operation.".into()})
    };
    Ok(
        json!({"scope":"claude_connections","status":status,"item":plan.item,"removed":removed,"retained_verified":retained,"message":message}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_unknown_duplicate_and_cross_assistant_selections_are_rejected() {
        for input in [
            "[]",
            "null",
            "{}",
            r#"["settings:PATH"]"#,
            r#"["codex:ANTHROPIC_API_KEY"]"#,
            r#"["settings:ANTHROPIC_API_KEY","settings:ANTHROPIC_API_KEY"]"#,
            r#"["user-env:HTTP_PROXY"]"#,
            r#"["shell:.zshrc:1:HTTP_PROXY"]"#,
        ] {
            assert!(selection(input).is_err(), "{input}");
        }
        for input in [
            r#"["settings:ANTHROPIC_CUSTOM_HEADERS"]"#,
            r#"["settings:root:apiKeyHelper"]"#,
            r#"["shell:.zshrc:2:ANTHROPIC_API_KEY"]"#,
            r#"["user-env:ANTHROPIC_API_KEY"]"#,
        ] {
            assert!(selection(input).is_ok());
        }
    }
}
