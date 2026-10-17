//! Exact Windows relative-path interpretation for materialized snapshots.
use anyhow::{bail, Result};
use std::collections::BTreeSet;

use super::snapshot_v2::{Entries, Entry};

fn component(name: &str) -> Result<()> {
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c <= '\u{1f}' || "\\/:<>\"|?*".contains(c))
    {
        bail!("Ambiguous Windows snapshot path component");
    }
    let stem = name
        .split('.')
        .next()
        .unwrap()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    }) {
        bail!("Reserved Windows device name in snapshot");
    }
    Ok(())
}

pub(super) fn key(path: &str) -> Result<()> {
    for part in path.split('/') {
        component(part)?;
    }
    Ok(())
}

fn directory(parts: &[String], entries: &Entries, aliases: &BTreeSet<&str>) -> Result<()> {
    let key = parts.join("/");
    if !parts.is_empty()
        && (aliases.contains(key.as_str()) || !matches!(entries.get(&key), Some(Entry::Directory)))
    {
        bail!("Materialized link traversal requires an included ordinary directory");
    }
    Ok(())
}

pub(super) fn target(
    path: &str,
    raw: &str,
    entries: &Entries,
    aliases: &BTreeSet<&str>,
) -> Result<String> {
    let resolved = resolve(path, raw, entries, aliases)?;
    if !matches!(entries.get(&resolved), Some(Entry::File(_))) {
        bail!("Materialized link target must be an included ordinary non-alias file");
    }
    Ok(resolved)
}

pub(super) fn resolve(
    path: &str,
    raw: &str,
    entries: &Entries,
    aliases: &BTreeSet<&str>,
) -> Result<String> {
    key(path)?;
    let mut parts: Vec<String> = path.split('/').map(str::to_string).collect();
    parts.pop();
    for step in raw.split(['/', '\\']) {
        directory(&parts, entries, aliases)?;
        match step {
            "." => {}
            ".." if parts.pop().is_some() => {}
            ".." => bail!("Materialized file link escapes its root"),
            _ => {
                component(step)?;
                parts.push(step.into());
            }
        }
    }
    let resolved = parts.join("/");
    if aliases.contains(resolved.as_str())
        || !matches!(
            entries.get(&resolved),
            Some(Entry::File(_) | Entry::Directory)
        )
    {
        bail!("Materialized link target must be an included ordinary non-alias file");
    }
    Ok(resolved)
}

pub(super) fn validate_keys(entries: &Entries) -> Result<()> {
    for path in entries.keys() {
        key(path)?;
    }
    Ok(())
}
