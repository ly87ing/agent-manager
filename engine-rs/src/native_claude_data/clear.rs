//! Confirmed, journaled cleanup. Failures retain verified progress and require a new preview.
use super::{files, preview::Snapshot, processes, tree};
use crate::{store, Context};
use anyhow::{bail, Result};
use cap_std::fs::Dir;
use serde_json::{json, Value};
use std::path::Path;

#[derive(Default)]
struct Progress {
    files: u64,
    directories: u64,
    credentials: u64,
}

impl Progress {
    fn data(&self, snapshot: &Snapshot, status: &str, verified: bool, message: &str) -> Value {
        json!({"scope":"default_claude_code","status":status,"deleted_files":self.files,
            "deleted_directories":self.directories,"deleted_credentials":self.credentials,
            "preserved_fields":snapshot.configurations.iter().map(|c|c.retained.fields).sum::<usize>(),
            "files_only":snapshot.files_only,"telemetry_verified":verified,"message":message})
    }
}

fn journal(
    conn: &rusqlite::Connection,
    run_id: &str,
    phase: &str,
    progress: &Progress,
) -> Result<()> {
    store::record_operation(
        conn,
        Some(run_id),
        "claude-data:clear",
        Some("default_claude_code"),
        Some(&json!({
            "phase":phase,"deleted_files":progress.files,"deleted_directories":progress.directories,
            "deleted_credentials":progress.credentials
        })),
    )
    .map_err(|_| anyhow::anyhow!("The cleanup audit could not be saved; cleanup stopped."))
}

fn guard(snapshot: &Snapshot) -> Result<()> {
    bindings(snapshot)?;
    if !processes::active()?.is_empty() {
        bail!("A Claude process appeared; cleanup stopped. Close it and request a new preview.");
    }
    Ok(())
}

fn bindings(snapshot: &Snapshot) -> Result<()> {
    let home_path = snapshot.root_path.parent().unwrap();
    let metadata = std::fs::symlink_metadata(home_path)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || tree::identity(&Dir::open_ambient_dir(
            home_path,
            cap_std::ambient_authority(),
        )?)? != tree::identity(&snapshot.home)?
    {
        bail!("The user HOME was replaced; cleanup stopped.");
    }
    match (&snapshot.root, snapshot.home.symlink_metadata(".claude")) {
        (Some(_), Ok(metadata)) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            if tree::identity(&snapshot.home.open_dir(".claude")?)? != snapshot.root_id {
                bail!("The Claude directory was replaced; cleanup stopped.");
            }
        }
        (None, Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
        _ => bail!("The Claude directory changed; cleanup stopped."),
    }
    Ok(())
}

fn parent(
    snapshot: &Snapshot,
    path: &Path,
    directories: &std::collections::BTreeMap<&Path, &tree::Entry>,
) -> Result<Dir> {
    let mut dir = snapshot
        .root
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("The Claude directory is unavailable."))?
        .try_clone()?;
    let mut prefix = std::path::PathBuf::new();
    for component in path.parent().unwrap_or(Path::new("")).components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            bail!("An item is outside the cleanup scope.");
        }
        prefix.push(component.as_os_str());
        let expected = directories
            .get(prefix.as_path())
            .ok_or_else(|| anyhow::anyhow!("A directory was not present in the preview."))?;
        let metadata = dir.symlink_metadata(component.as_os_str())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            bail!("A directory was redirected; cleanup stopped.");
        }
        let next = dir.open_dir(component.as_os_str())?;
        if tree::identity(&next)? != expected.identity {
            bail!("A directory was replaced; cleanup stopped.");
        }
        dir = next;
    }
    Ok(dir)
}

fn telemetry_verified(snapshot: &Snapshot) -> bool {
    if bindings(snapshot).is_err() {
        return false;
    }
    use std::io::Read;
    snapshot.configurations.iter().all(|config| {
        let dir = if config.in_root {
            snapshot.root.as_ref().unwrap()
        } else {
            &snapshot.home
        };
        let Some(expected) = &config.retained.bytes else {
            return true;
        };
        let Ok(entry) = tree::entry(dir, Path::new(&config.name), config.name.clone().into())
        else {
            return false;
        };
        if entry.kind != tree::Kind::File {
            return false;
        }
        let Ok(mut file) = dir.open(&config.name) else {
            return false;
        };
        let mut actual = Vec::new();
        if file
            .by_ref()
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut actual)
            .is_err()
        {
            return false;
        }
        let Ok(retained) = super::telemetry::retain(&actual, &snapshot.root_path.to_string_lossy())
        else {
            return false;
        };
        retained
            .bytes
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            == serde_json::from_slice::<Value>(expected).ok()
            && tree::entry(dir, Path::new(&config.name), config.name.clone().into())
                .is_ok_and(|after| after.hash == entry.hash)
    })
}

fn apply(
    snapshot: &Snapshot,
    progress: &mut Progress,
    mut check: impl FnMut() -> Result<()>,
    mut audit: impl FnMut(&str, &Progress) -> Result<()>,
) -> Result<()> {
    check()?;
    audit("credentials", progress)?;
    snapshot
        .credentials
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Credential evidence is unavailable."))?
        .clear(&mut check, || {
            progress.credentials += 1;
            audit("credential_removed", progress)
        })?;
    check()?;
    audit("configurations", progress)?;
    for config in &snapshot.configurations {
        check()?;
        if !config.changed {
            continue;
        }
        let dir = if config.in_root {
            snapshot.root.as_ref().unwrap()
        } else {
            &snapshot.home
        };
        if let Some(bytes) = &config.retained.bytes {
            let absolute = if config.in_root {
                snapshot.root_path.as_path()
            } else {
                snapshot.root_path.parent().unwrap()
            };
            files::replace(dir, absolute, Path::new(&config.name), &config.entry, bytes)?;
        } else {
            files::remove(dir, Path::new(&config.name), &config.entry)?;
            progress.files += 1;
        }
        audit("configuration_processed", progress)?;
    }
    audit("files", progress)?;
    let directories = snapshot
        .entries
        .iter()
        .filter(|e| e.kind == tree::Kind::Directory)
        .map(|e| (e.path.as_path(), e))
        .collect();
    for entry in &snapshot.entries {
        if snapshot
            .configurations
            .iter()
            .any(|c| c.in_root && c.entry.path == entry.path)
        {
            continue;
        }
        // Process checks are bounded by phases and directory boundaries; every leaf is re-read.
        if entry.kind == tree::Kind::Directory {
            check()?;
        }
        bindings(snapshot)?;
        let dir = parent(snapshot, &entry.path, &directories)?;
        files::remove(&dir, Path::new(entry.path.file_name().unwrap()), entry)?;
        if entry.kind == tree::Kind::Directory {
            progress.directories += 1;
        } else {
            progress.files += 1;
        }
    }
    check()?;
    Ok(())
}

pub(super) fn execute(ctx: &Context, base: &str, files_only: bool, run_id: &str) -> Result<Value> {
    let initial = Snapshot::capture(ctx, files_only)?;
    if initial.base != base {
        bail!("The preview is stale; no cleanup was started. Request a new preview.");
    }
    if !initial.blockers.is_empty() {
        bail!("The preview contains blockers; no cleanup was started.");
    }
    if !initial.data()["has_changes"].as_bool().unwrap_or(false) {
        return Ok(Progress::default().data(
            &initial,
            "not_applied",
            true,
            "There is no data to clear in the selected scope.",
        ));
    }
    drop(initial);
    let _lock = crate::native::acquire_apply_lock(ctx, "Claude local data maintenance")?;
    let snapshot = Snapshot::capture(ctx, files_only)?;
    if snapshot.base != base || !snapshot.blockers.is_empty() {
        bail!("The preview changed before cleanup; request a new preview.");
    }
    let conn = store::open_write(ctx).map_err(|_| {
        anyhow::anyhow!("The cleanup audit is unavailable; no cleanup was started.")
    })?;
    let mut progress = Progress::default();
    journal(&conn, run_id, "intent", &progress)?;
    let result = apply(
        &snapshot,
        &mut progress,
        || guard(&snapshot),
        |phase, p| journal(&conn, run_id, phase, p),
    );
    let verified = telemetry_verified(&snapshot);
    let final_check = if result.is_ok() && verified {
        Snapshot::capture(ctx, files_only).and_then(|after| {
            if !after.blockers.is_empty() || after.data()["has_changes"].as_bool() != Some(false) {
                bail!("New or remaining local data was found; request a new preview.");
            }
            Ok(())
        })
    } else {
        Ok(())
    };
    let failure = result
        .err()
        .or_else(|| final_check.err())
        .map(|e| e.to_string());
    let phase = if failure.is_none() && verified {
        "complete"
    } else {
        "partial"
    };
    let audit_error = journal(&conn, run_id, phase, &progress).err();
    let status = if phase == "complete" && audit_error.is_none() {
        "complete"
    } else {
        "partial"
    };
    let message = failure.or_else(||audit_error.map(|e|e.to_string())).unwrap_or_else(||
        if verified {"The selected local scope was independently checked. Remote tokens and shared SDK credentials were not revoked.".into()}
        else {"Telemetry readback is uncertain; inspect the remaining files before requesting a new preview.".into()});
    Ok(progress.data(&snapshot, status, verified, &message))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (std::path::PathBuf, Context) {
        let path = std::env::temp_dir().join(format!("am-claude-clear-{}", store::uuid_v4()));
        let ctx = Context {
            home_dir: path.join("home"),
            data_dir: path.join("manager"),
        };
        std::fs::create_dir_all(ctx.home_dir.join(".claude/projects/project/memory")).unwrap();
        for (name, bytes) in [
            ("projects/project/session.jsonl", "PRIVATE_SESSION"),
            ("projects/project/memory/data.md", "PRIVATE_MEMORY"),
            (".credentials.json", "PRIVATE_ACCOUNT"),
        ] {
            std::fs::write(ctx.home_dir.join(".claude").join(name), bytes).unwrap();
        }
        std::fs::write(ctx.home_dir.join(".claude/settings.json"),br#"{"env":{"DISABLE_TELEMETRY":"0","OTEL_EXPORTER_OTLP_HEADERS":"PRIVATE_TELEMETRY"},"model":"old"}"#).unwrap();
        std::fs::write(
            ctx.home_dir.join(".claude.json"),
            br#"{"userID":"PRIVATE_ACCOUNT_ID","oauthAccount":{"emailAddress":"PRIVATE_EMAIL"}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(ctx.home_dir.join("project/.claude")).unwrap();
        std::fs::write(
            ctx.home_dir.join("project/.claude/settings.json"),
            "PROJECT_SOURCE",
        )
        .unwrap();
        (path, ctx)
    }
    #[test]
    fn confirmed_cleanup_deletes_sessions_accounts_and_memory_and_reads_back_retained_telemetry() {
        let (path, ctx) = fixture();
        let snapshot = Snapshot::capture(&ctx, true).unwrap();
        assert!(snapshot.blockers.is_empty(), "{:?}", snapshot.blockers);
        let result = execute(&ctx, &snapshot.base, true, "fixture").unwrap();
        assert_eq!(result["status"], "complete", "{result}");
        assert_eq!(result["deleted_files"], 4);
        assert_eq!(result["deleted_directories"], 3);
        assert_eq!(result["telemetry_verified"], true);
        let retained: Value = serde_json::from_slice(
            &std::fs::read(ctx.home_dir.join(".claude/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            retained,
            json!({"env":{"DISABLE_TELEMETRY":"0","OTEL_EXPORTER_OTLP_HEADERS":"PRIVATE_TELEMETRY"}})
        );
        assert_eq!(
            std::fs::read_to_string(ctx.home_dir.join("project/.claude/settings.json")).unwrap(),
            "PROJECT_SOURCE"
        );
        assert!(!ctx.home_dir.join(".claude.json").exists());
        assert!(
            !Snapshot::capture(&ctx, true).unwrap().data()["has_changes"]
                .as_bool()
                .unwrap()
        );
        let conn = store::open_readonly(&ctx).unwrap().unwrap();
        let details: String = conn
            .query_row(
                "SELECT group_concat(detail_json) FROM operation_logs",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!details.contains("PRIVATE_"));
        drop(conn);
        drop(snapshot);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn stale_preview_is_refused_before_creating_a_database_or_removing_anything() {
        let (path, ctx) = fixture();
        let snapshot = Snapshot::capture(&ctx, true).unwrap();
        std::fs::write(
            ctx.home_dir.join(".claude/.credentials.json"),
            "CHANGED_ACCOUNT",
        )
        .unwrap();
        assert!(execute(&ctx, &snapshot.base, true, "fixture").is_err());
        assert!(!ctx.data_dir.exists());
        assert!(ctx
            .home_dir
            .join(".claude/projects/project/session.jsonl")
            .exists());
        drop(snapshot);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn a_new_file_mid_cleanup_retains_prior_effects_and_is_not_removed() {
        let (path, ctx) = fixture();
        let snapshot = Snapshot::capture(&ctx, true).unwrap();
        let mut progress = Progress::default();
        let result = apply(
            &snapshot,
            &mut progress,
            || Ok(()),
            |phase, _| {
                if phase == "files" {
                    std::fs::write(
                        ctx.home_dir.join(".claude/projects/project/new.jsonl"),
                        "NEW_WRITER",
                    )?;
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(ctx.home_dir.join(".claude/projects/project/new.jsonl"))
                .unwrap(),
            "NEW_WRITER"
        );
        assert!(progress.files >= 1);
        assert!(telemetry_verified(&snapshot));
        drop(snapshot);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn a_rebound_root_stops_before_deleting_leaves_from_the_retained_directory() {
        let (path, ctx) = fixture();
        let snapshot = Snapshot::capture(&ctx, true).unwrap();
        let mut progress = Progress::default();
        let result = apply(
            &snapshot,
            &mut progress,
            || Ok(()),
            |phase, _| {
                if phase == "files" {
                    std::fs::rename(ctx.home_dir.join(".claude"), ctx.home_dir.join("retained"))?;
                    std::fs::create_dir(ctx.home_dir.join(".claude"))?;
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(ctx.home_dir.join("retained/.credentials.json")).unwrap(),
            "PRIVATE_ACCOUNT"
        );
        assert_eq!(
            std::fs::read_to_string(ctx.home_dir.join("retained/projects/project/session.jsonl"))
                .unwrap(),
            "PRIVATE_SESSION"
        );
        drop(snapshot);
        std::fs::remove_dir_all(path).unwrap();
    }
}
