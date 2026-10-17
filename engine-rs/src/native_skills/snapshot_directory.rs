//! Ordinary sibling-directory copies anchored to an unchanged logical typed ID.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::snapshot_materialized::FileLink;
use super::snapshot_materialized_path::{key, resolve, validate_keys};
use super::snapshot_v2::{typed_hash, Entries, Entry};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DirectoryLink {
    pub(super) path: String,
    pub(super) raw_target: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub(super) schema: u32,
    pub(super) version: String,
    pub(super) materialized_file_links: Vec<FileLink>,
    pub(super) materialized_directory_links: Vec<DirectoryLink>,
}

pub(super) struct Plan {
    pub(super) manifest: Vec<u8>,
    pub(super) paths: BTreeMap<String, String>,
}

fn under(path: &str, root: &str) -> bool {
    path.strip_prefix(root)
        .is_some_and(|tail| tail.starts_with('/'))
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("")
}

fn portable_keys(entries: &Entries) -> Result<()> {
    validate_keys(entries)?;
    let mut budget = super::snapshot_budget::Budget::default();
    let mut names = BTreeSet::new();
    for path in entries.keys() {
        budget.entry(path, path)?;
        if path.chars().any(|c| {
            !c.is_ascii()
                && (c.to_uppercase().ne(std::iter::once(c))
                    || c.to_lowercase().ne(std::iter::once(c)))
        }) {
            bail!("Directory materialization cannot establish portable non-ASCII case mapping");
        }
        if !names.insert(path.to_ascii_lowercase()) {
            bail!("Directory materialization contains case-colliding paths");
        }
    }
    Ok(())
}

fn aliases(manifest: &Manifest) -> Result<BTreeSet<&str>> {
    if manifest.materialized_directory_links.is_empty() {
        bail!("Directory materialization requires directory records");
    }
    let mut aliases = BTreeSet::new();
    for path in manifest
        .materialized_file_links
        .iter()
        .map(|link| link.path.as_str())
        .chain(
            manifest
                .materialized_directory_links
                .iter()
                .map(|link| link.path.as_str()),
        )
    {
        key(path)?;
        if !aliases.insert(path) {
            bail!("Duplicate materialized alias path");
        }
    }
    for path in &aliases {
        let mut ancestor = parent(path);
        while !ancestor.is_empty() {
            if aliases.contains(ancestor) {
                bail!("Materialized aliases must not overlap");
            }
            ancestor = parent(ancestor);
        }
    }
    Ok(aliases)
}

fn directory_target(
    link: &DirectoryLink,
    entries: &Entries,
    aliases: &BTreeSet<&str>,
) -> Result<String> {
    let target = resolve(&link.path, &link.raw_target, entries, aliases)?;
    if parent(&target) != parent(&link.path)
        || !matches!(entries.get(&target), Some(Entry::Directory))
    {
        bail!("Directory materialization requires an included ordinary sibling target");
    }
    if aliases.iter().any(|alias| under(alias, &target)) {
        bail!("Materialized directory targets must not contain aliases");
    }
    Ok(target)
}

fn subtree<'a>(entries: &'a Entries, root: &'a str) -> impl Iterator<Item = (&'a str, &'a Entry)> {
    let prefix = format!("{root}/");
    entries
        .range(prefix.clone()..)
        .take_while(move |(path, _)| path.starts_with(&prefix))
        .map(move |(path, entry)| (&path[root.len()..], entry))
}

pub(super) fn plan(entries: &Entries) -> Result<Option<Plan>> {
    if !entries.values().any(|entry| matches!(entry, Entry::Link {target_relative, ..} if matches!(entries.get(target_relative), Some(Entry::Directory)))) {
        return Ok(None);
    }
    portable_keys(entries)?;
    let mut manifest = Manifest {
        schema: 4,
        version: typed_hash(entries),
        materialized_file_links: Vec::new(),
        materialized_directory_links: Vec::new(),
    };
    for (path, entry) in entries {
        if let Entry::Link {
            raw_target,
            target_relative,
        } = entry
        {
            match entries.get(target_relative) {
                Some(Entry::File(_)) => manifest.materialized_file_links.push(FileLink {
                    path: path.clone(),
                    raw_target: raw_target.clone(),
                }),
                Some(Entry::Directory) => {
                    manifest.materialized_directory_links.push(DirectoryLink {
                        path: path.clone(),
                        raw_target: raw_target.clone(),
                    })
                }
                _ => bail!("Materialized targets must be direct ordinary entries"),
            }
        }
    }
    let aliases = aliases(&manifest)?;
    let mut resolved = BTreeMap::new();
    for link in &manifest.materialized_file_links {
        let target = super::snapshot_materialized_path::target(
            &link.path,
            &link.raw_target,
            entries,
            &aliases,
        )?;
        resolved.insert(link.path.as_str(), target);
    }
    for link in &manifest.materialized_directory_links {
        let target = directory_target(link, entries, &aliases)?;
        if subtree(entries, &target).any(|(_, entry)| matches!(entry, Entry::Link { .. })) {
            bail!("Directory targets must contain only ordinary entries");
        }
        resolved.insert(link.path.as_str(), target);
    }
    let mut budget = super::snapshot_budget::Budget::default();
    let mut physical_budget = super::snapshot_budget::Budget::default();
    let mut paths = BTreeMap::new();
    let mut insert = |output: String, input: String| -> Result<()> {
        budget.entry(&output, &input)?;
        physical_budget.entry(&output, &output)?;
        if paths.insert(output, input).is_some() {
            bail!("Overlapping materialized output paths");
        }
        Ok(())
    };
    for (path, entry) in entries {
        if let Entry::Link {
            target_relative, ..
        } = entry
        {
            let target = resolved.get(path.as_str()).unwrap();
            if target != target_relative {
                bail!("Materialization target differs from its logical source");
            }
            insert(path.clone(), target.clone())?;
            if matches!(entries.get(target), Some(Entry::Directory)) {
                for (suffix, _) in subtree(entries, target) {
                    insert(format!("{path}{suffix}"), format!("{target}{suffix}"))?;
                }
            }
        } else {
            insert(path.clone(), path.clone())?;
        }
    }
    let manifest = serde_json::to_vec(&manifest)?;
    if manifest.len() > 64 * 1024 {
        bail!("Directory materialization manifest exceeds 64 KiB");
    }
    Ok(Some(Plan { manifest, paths }))
}

pub(super) fn verify(mut entries: Entries, manifest: &Manifest) -> Result<String> {
    portable_keys(&entries)?;
    if entries
        .values()
        .any(|entry| matches!(entry, Entry::Link { .. }))
    {
        bail!("Materialized snapshot contains physical links");
    }
    let aliases = aliases(manifest)?;
    let mut replacements = Vec::new();
    let mut removals = Vec::new();
    for link in &manifest.materialized_directory_links {
        let target = directory_target(link, &entries, &aliases)?;
        if !matches!(entries.get(&link.path), Some(Entry::Directory))
            || !subtree(&entries, &link.path).eq(subtree(&entries, &target))
        {
            bail!("Materialized directory contents differ from their ordinary target");
        }
        removals.extend(
            subtree(&entries, &link.path).map(|(suffix, _)| format!("{}{suffix}", link.path)),
        );
        replacements.push((
            link.path.clone(),
            Entry::Link {
                raw_target: link.raw_target.clone(),
                target_relative: target,
            },
        ));
    }
    for link in &manifest.materialized_file_links {
        let target = super::snapshot_materialized_path::target(
            &link.path,
            &link.raw_target,
            &entries,
            &aliases,
        )?;
        match (entries.get(&link.path), entries.get(&target)) {
            (Some(Entry::File(a)), Some(Entry::File(b))) if a == b => {}
            _ => bail!("Materialized file bytes differ from their ordinary target"),
        }
        replacements.push((
            link.path.clone(),
            Entry::Link {
                raw_target: link.raw_target.clone(),
                target_relative: target,
            },
        ));
    }
    for path in removals {
        entries.remove(&path);
    }
    entries.extend(replacements);
    Ok(typed_hash(&entries))
}

pub(super) fn preflight(
    source: &Path,
    destination: &Path,
    entries: &Entries,
    plan: &Plan,
) -> Result<()> {
    super::snapshot_budget::preflight(source, destination, &plan.paths, entries)
}

#[cfg(test)]
#[path = "snapshot_directory_tests.rs"]
mod tests;
