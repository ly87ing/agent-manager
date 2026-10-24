//! Keep source-file opens outside the process that owns Manager's SQLite locks.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

const PROTOCOL: &str = "orca-search-inspection-v1";
const FLAG: &str = "--inspect-orca-search";
const LIMIT: usize = 4096;
static HELPER: OnceLock<Option<PathBuf>> = OnceLock::new();

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Counts {
    claude_sessions: u64,
    claude_messages: u64,
    claude_file_cursors: u64,
    non_claude_sessions: u64,
    unattributed_messages: u64,
    unattributed_file_cursors: u64,
    unattributed_fts_rows: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    protocol: String,
    status: String,
    #[serde(deserialize_with = "Option::<Counts>::deserialize")]
    counts: Option<Counts>,
}

pub(crate) fn configure(path: Option<PathBuf>) {
    let _ = HELPER.set(path);
}

pub(super) fn inspect(path: &Path) -> Result<Option<Value>> {
    inspect_with_helper(path, &helper_path(HELPER.get())?)
}

fn helper_path(configured: Option<&Option<PathBuf>>) -> Result<PathBuf> {
    match configured {
        Some(Some(path)) => Ok(path.clone()),
        Some(None) => bail!("Inspection component location is unavailable"),
        None => Ok(std::env::current_exe()?
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Missing executable directory"))?
            .join(format!(
                "agent-manager-claude-data-helper{}",
                std::env::consts::EXE_SUFFIX
            ))),
    }
}

fn inspect_with_helper(path: &Path, helper: &Path) -> Result<Option<Value>> {
    // Do not open/hash the source (or another possible database alias) here.
    // Only the packaged executable's ordinary-file metadata is inspected.
    let metadata = std::fs::symlink_metadata(helper)?;
    if !helper.is_absolute() || !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("Inspection component is unavailable");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            bail!("Redirected inspection component");
        }
    }
    let mut command = crate::process::launch_command(helper);
    command
        .args([std::ffi::OsStr::new(FLAG), path.as_os_str()])
        .current_dir(helper.parent().unwrap())
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // Windows loader/runtime essentials only; no credentials or proxy discovery.
    for key in ["SystemRoot", "WINDIR", "TMP", "TEMP", "TMPDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let output = crate::process::capture_group(&mut command, Duration::from_secs(20), LIMIT)?;
    if !output.status.success() || output.stdout.len() >= LIMIT {
        bail!("Inspection component did not return a bounded result");
    }
    parse(&output.stdout)
}

fn parse(bytes: &[u8]) -> Result<Option<Value>> {
    let reply: Reply = serde_json::from_slice(bytes)?;
    if reply.protocol != PROTOCOL {
        bail!("Unsupported inspection component protocol");
    }
    match (reply.status.as_str(), reply.counts) {
        ("inspected", Some(counts)) => Ok(Some(serde_json::to_value(counts)?)),
        ("not_found", None) => Ok(None),
        _ => bail!("Inspection is unavailable"),
    }
}

/// Separate executable only: no general dispatcher, app storage or credentials.
pub(crate) fn helper_main(args: &[OsString]) -> i32 {
    if args.len() != 2 || args[0] != FLAG {
        return 2;
    }
    let path = Path::new(&args[1]);
    if !path.is_absolute()
        || path
            .file_name()
            .is_none_or(|name| name != super::orca_search::DATABASE)
        || path.to_str().is_none_or(|s| s.len() > 256 * 1024)
    {
        return 2;
    }
    let (status, counts) = match super::orca_search::inspect(path) {
        Ok(Some(value)) => match serde_json::from_value(value) {
            Ok(counts) => ("inspected", Some(counts)),
            Err(_) => ("unavailable", None),
        },
        Ok(None) => ("not_found", None),
        Err(_) => ("unavailable", None),
    };
    let reply = Reply {
        protocol: PROTOCOL.into(),
        status: status.into(),
        counts,
    };
    match serde_json::to_string(&reply) {
        Ok(json) => {
            println!("{json}");
            0
        }
        Err(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refused_desktop_resource_path_never_uses_the_cli_fallback() {
        assert!(helper_path(Some(&None)).is_err());
        let path = Some(std::env::temp_dir().join("SYNTHETIC_COMPONENT"));
        assert_eq!(helper_path(Some(&path)).unwrap(), path.unwrap());
    }

    #[test]
    fn response_requires_exact_protocol_and_count_shape() {
        let valid =
            br#"{"protocol":"orca-search-inspection-v1","status":"not_found","counts":null}"#;
        assert!(parse(valid).unwrap().is_none());
        for invalid in [
            br#"{"protocol":"unsupported","status":"not_found","counts":null}"#.as_slice(),
            br#"{"protocol":"orca-search-inspection-v1","status":"not_found"}"#.as_slice(),
            br#"{"protocol":"orca-search-inspection-v1","status":"not_found","counts":null,"counts":null}"#.as_slice(),
            br#"{"protocol":"orca-search-inspection-v1","status":"inspected","counts":null}"#.as_slice(),
            br#"{"protocol":"orca-search-inspection-v1","status":"not_found","counts":null,"body":"SYNTHETIC_PRIVATE"}"#.as_slice(),
        ] {
            assert!(parse(invalid).is_err());
        }
    }

    #[test]
    fn helper_rejects_other_leaf_names_before_inspection() {
        let root =
            std::env::temp_dir().join(format!("am-inspection-args-{}", crate::store::uuid_v4()));
        assert_eq!(
            helper_main(&[FLAG.into(), root.join("other.sqlite").into_os_string()]),
            2
        );
        assert_eq!(
            helper_main(&[
                FLAG.into(),
                root.join("session-search.sqlite").into_os_string(),
                "--confirm".into()
            ]),
            2
        );
        assert!(!root.exists());
    }

    #[test]
    fn missing_component_is_unknown_even_for_an_absent_source() {
        let root =
            std::env::temp_dir().join(format!("am-inspection-missing-{}", crate::store::uuid_v4()));
        assert!(inspect_with_helper(&root.join("source"), &root.join("helper")).is_err());
        assert!(!root.exists());
    }
}
