//! Installed Git origin, main-tool receipt and command evidence; never execute a candidate.
use super::uv_metadata_files::Directory;
use super::uv_postcondition::{UnverifiedEntry, UnverifiedVersion};
use crate::native::Context;
use crate::platform::Canon;
use anyhow::{anyhow, bail, Context as _, Result};
use serde::Deserialize;
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Origin {
    url: String,
    vcs_info: Vcs,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Vcs {
    vcs: String,
    commit_id: String,
    requested_revision: Option<String>,
    #[serde(rename = "git_lfs")]
    _git_lfs: Option<bool>,
}

fn repository(url: &str) -> Option<String> {
    let (host, path) = url.strip_prefix("https://")?.split_once('/')?;
    let path = path.strip_suffix(".git").unwrap_or(path);
    if host.is_empty()
        || path.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || !path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
    {
        return None;
    }
    Some(format!("https://{}/{path}", host.to_ascii_lowercase()))
}

pub(super) fn target(source: &str, commit: &str) -> Result<String> {
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("Managed Git dependencies require a full 40-character commit ID");
    }
    repository(&format!("https://{source}")).context("Unsupported Git repository identity")
}

pub(super) fn absent(directory: &Path) -> Result<()> {
    match std::fs::symlink_metadata(directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => Err(UnverifiedVersion(format!("Incomplete Git candidate exists at {}. It is retained; inspect and move it explicitly before retrying.", directory.display())).into()),
    }
}

fn receipt(directory: &Directory) -> Result<toml::Value> {
    let mut bytes = Vec::new();
    directory
        .file(OsStr::new("uv-receipt.toml"))?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 {
        bail!("Tool receipt exceeds the read limit");
    }
    std::str::from_utf8(&bytes)?
        .parse()
        .map_err(|_| anyhow!("Tool receipt is malformed or contains duplicate fields"))
}

fn metadata(
    ctx: &Context,
    directory: &Path,
    expected: &str,
    commit: &str,
) -> Result<(PathBuf, toml::Value)> {
    let root = Directory::open(directory)?;
    let tools = root.child(OsStr::new("tools"))?;
    let mut selected = None;
    for entry in tools.entries(32)? {
        if [".lock", ".gitignore", "CACHEDIR.TAG"]
            .iter()
            .any(|name| entry == OsStr::new(name))
        {
            tools.file(&entry)?;
            continue;
        }
        let name = entry.to_str().context("Non-UTF-8 tool name")?;
        let normalized = super::uv_metadata::normalized_name(name).context("Invalid tool name")?;
        let environment = tools.child(&entry)?;
        if selected.is_some() {
            bail!("Ambiguous Git tool environment");
        }
        let receipt = receipt(&environment)?;
        let tool = receipt.get("tool").context("Missing tool receipt")?;
        let requirements = tool
            .get("requirements")
            .and_then(toml::Value::as_array)
            .context("Missing tool requirement")?;
        if requirements.len() != 1
            || requirements[0]
                .get("name")
                .and_then(toml::Value::as_str)
                .and_then(super::uv_metadata::normalized_name)
                .as_deref()
                != Some(&normalized)
            || requirements[0]
                .get("git")
                .and_then(toml::Value::as_str)
                .is_none()
        {
            bail!("Git receipt does not identify the main tool");
        }
        let environment_path = directory.join("tools").join(&entry);
        let origin: Origin = super::uv_metadata::origin(&environment_path, &normalized)?;
        if repository(&origin.url).as_deref() != Some(expected)
            || origin.vcs_info.vcs != "git"
            || !origin.vcs_info.commit_id.eq_ignore_ascii_case(commit)
            || origin
                .vcs_info
                .requested_revision
                .as_ref()
                .is_some_and(|revision| !revision.eq_ignore_ascii_case(commit))
        {
            bail!("Installed Git source or commit disagrees with the requested candidate");
        }
        let entries = tool
            .get("entrypoints")
            .and_then(toml::Value::as_array)
            .context("Missing tool entries")?;
        let names = entries
            .iter()
            .map(|entry| {
                entry
                    .get("name")
                    .and_then(toml::Value::as_str)
                    .context("Invalid tool entry name")
            })
            .collect::<Result<Vec<_>>>()?;
        super::scope::Scope::new(&crate::store::effective_policy(ctx)?)
            .ensure_extension(&format!("pypi:{normalized}"), names.iter().copied())?;
        selected = Some((environment_path, receipt));
    }
    selected.context("Git tool environment is unavailable")
}

#[cfg(windows)]
fn same_bytes(left: &Path, right: &Path) -> Result<bool> {
    let open = |path: &Path| -> Result<std::fs::File> {
        let parent = Directory::open(&path.parent().context("Missing entry parent")?.canon()?)?;
        Ok(parent.file(path.file_name().context("Missing entry name")?)?)
    };
    let (mut left, mut right) = (open(left)?, open(right)?);
    if left.metadata()?.len() > 64 * 1024 * 1024
        || left.metadata()?.len() != right.metadata()?.len()
    {
        return Ok(false);
    }
    let (mut a, mut b) = (vec![0; 64 * 1024], vec![0; 64 * 1024]);
    let mut remaining = 64 * 1024 * 1024;
    loop {
        let count = left.read(&mut a)?;
        if count == 0 {
            return Ok(right.read(&mut b[..1])? == 0);
        }
        if count > remaining {
            return Ok(false);
        }
        remaining -= count;
        right.read_exact(&mut b[..count])?;
        if a[..count] != b[..count] {
            return Ok(false);
        }
    }
}

fn entries(
    directory: &Path,
    environment: &Path,
    receipt: &toml::Value,
    bins: &[String],
    home: &Path,
    stage: &str,
) -> Result<()> {
    let declarations = receipt
        .get("tool")
        .and_then(|tool| tool.get("entrypoints"))
        .and_then(toml::Value::as_array)
        .context("Missing Git tool entries")?;
    if bins.is_empty() {
        bail!("Missing declared Git commands");
    }
    let mut actual = Vec::new();
    let mut sources = Vec::new();
    for bin in bins {
        let filename = if cfg!(windows) {
            format!("{bin}.exe")
        } else {
            bin.clone()
        };
        let outer = directory.join("bin").join(&filename);
        let source = environment
            .join(if cfg!(windows) { "Scripts" } else { "bin" })
            .join(filename);
        let matching = declarations
            .iter()
            .filter(|entry| entry.get("name").and_then(toml::Value::as_str) == Some(bin.as_str()))
            .collect::<Vec<_>>();
        if matching.len() != 1
            || matching[0]
                .get("install-path")
                .and_then(toml::Value::as_str)
                .and_then(|path| Path::new(path).canon().ok())
                != Some(outer.canon()?)
        {
            bail!("Git command is not bound to its tool receipt");
        }
        // Older uv receipts omit the provider. A recorded provider must match the main tool.
        if matching[0].get("from").is_some_and(|value| {
            value.as_str().and_then(super::uv_metadata::normalized_name)
                != environment
                    .file_name()
                    .and_then(OsStr::to_str)
                    .and_then(super::uv_metadata::normalized_name)
        }) {
            bail!("Git command receipt identifies another provider distribution");
        }
        actual.push(outer);
        sources.push(source);
    }
    super::uv_postcondition::check_entries(directory, &actual, home, stage)?;
    super::uv_postcondition::check_entries(environment, &sources, home, stage)?;
    for (actual, source) in actual.iter().zip(&sources) {
        #[cfg(unix)]
        if actual.canon()? != source.canon()? {
            bail!("Git entry targets another tool");
        }
        #[cfg(windows)]
        if !same_bytes(actual, source)? {
            bail!("Git entry bytes differ from the main tool command");
        }
    }
    Ok(())
}

pub(super) fn check(
    ctx: &Context,
    directory: &Path,
    source: &str,
    commit: &str,
    bins: &[String],
    stage: &str,
) -> Result<()> {
    // Normalize aliases in parents, while requiring the candidate itself to be ordinary.
    let inspected = (|| -> Result<_> {
        let root = directory
            .parent()
            .context("Missing Git candidate parent")?
            .canon()?
            .join(
                directory
                    .file_name()
                    .context("Missing Git candidate name")?,
            );
        let (environment, receipt) = metadata(ctx, &root, &target(source, commit)?, commit)?;
        Ok((root, environment, receipt))
    })();
    let (root, environment, receipt) = inspected.map_err(|error| anyhow!(UnverifiedVersion(format!(
        "Managed Git source/commit verification before {stage} failed: {error:#}. This step was not performed. Candidate {} is retained without automatic restoration or cleanup after this check.", directory.display()
    ))))?;
    entries(&root, &environment, &receipt, bins, &ctx.home_dir, stage).map_err(|error| anyhow!(UnverifiedEntry(format!(
        "Managed Git command verification before {stage} failed: {error:#}. This step was not performed. Candidate {} is retained for inspection without automatic restoration or cleanup after this check.", directory.display()
    ))))
}
