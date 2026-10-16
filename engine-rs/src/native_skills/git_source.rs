//! Git working-tree kinds, independent of immutable snapshot verification.
use crate::platform::Canon;
use anyhow::{anyhow, bail, Result};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::snapshot_v2::{self, Entries, Entry, SourceVersions};

pub(super) struct Source {
    root: PathBuf,
    original_root: PathBuf,
    root_guard: Option<(std::fs::File, super::snapshot_storage::DirectoryId)>,
    index: Result<super::git_index::Index, String>,
}

impl Source {
    pub(super) fn new(root: &Path) -> Self {
        let inspected = (|| -> Result<_> {
            let (handle, id) = super::snapshot_storage::open_directory(root)?;
            let index = super::git_index::read(root)?;
            let resolved = root.canon()?;
            super::snapshot_storage::unchanged(root, id)?;
            super::snapshot_storage::unchanged(&resolved, id)?;
            let mut names = BTreeMap::new();
            let mut budget = 0;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            for path in index.links.iter().chain(&index.gitlinks) {
                exact_path(
                    &resolved,
                    std::str::from_utf8(path)?,
                    &mut names,
                    &mut budget,
                    deadline,
                )?;
            }
            super::snapshot_storage::unchanged(root, id)?;
            Ok((resolved, index, handle, id))
        })();
        match inspected {
            Ok((resolved, index, handle, id)) => Self {
                root: resolved,
                original_root: root.into(),
                root_guard: Some((handle, id)),
                index: Ok(index),
            },
            Err(error) => Self {
                root: root.into(),
                original_root: root.into(),
                root_guard: None,
                index: Err(error.to_string()),
            },
        }
    }

    pub(super) fn error(&self) -> Option<&str> {
        self.index.as_ref().err().map(String::as_str)
    }

    fn inventory(&self, skill: &Path) -> Result<(Entries, SourceVersions, bool)> {
        if let Some((_, id)) = &self.root_guard {
            super::snapshot_storage::unchanged(&self.original_root, *id)?;
            super::snapshot_storage::unchanged(&self.root, *id)?;
        }
        let (_handle, id) = super::snapshot_storage::open_directory(skill)?;
        let resolved = skill.canon()?;
        let result = self.inventory_entries(&resolved)?;
        super::snapshot_storage::unchanged(skill, id)?;
        super::snapshot_storage::unchanged(&resolved, id)?;
        if let Some((_, id)) = &self.root_guard {
            super::snapshot_storage::unchanged(&self.original_root, *id)?;
        }
        Ok(result)
    }

    fn inventory_entries(&self, skill: &Path) -> Result<(Entries, SourceVersions, bool)> {
        let index = self
            .index
            .as_ref()
            .map_err(|error| anyhow!("Git source kinds are unknown: {error}"))?;
        let prefix = snapshot_v2::relative_key(skill.strip_prefix(&self.root)?)?;
        exact_path(
            &self.root,
            &prefix,
            &mut BTreeMap::new(),
            &mut 0,
            std::time::Instant::now() + std::time::Duration::from_secs(30),
        )?;
        for gitlink in &index.gitlinks {
            if overlaps(prefix.as_bytes(), gitlink) {
                bail!("Git skill overlaps a submodule whose internal entry kinds are unknown");
            }
        }
        let mut entries = snapshot_v2::source_entries(skill)?;
        if !index.links.is_empty() {
            let scope = if prefix.is_empty() {
                String::new()
            } else {
                format!("{prefix}/")
            };
            for link in &index.links {
                let Some(relative) = link.strip_prefix(scope.as_bytes()) else {
                    continue;
                };
                let relative = std::str::from_utf8(relative)?;
                let parts: Vec<_> = relative.split('/').collect();
                if parts[..parts.len().saturating_sub(1)]
                    .iter()
                    .any(|part| super::EXCLUDED_DIRS.contains(part))
                {
                    continue;
                }
                if !entries.contains_key(relative) {
                    bail!("An indexed Git link is missing or has a different path spelling");
                }
            }
        }
        let mut versions = snapshot_v2::entry_versions(&entries);
        let mut plaintext = false;
        for (path, entry) in &mut entries {
            let key = if prefix.is_empty() {
                path.clone()
            } else {
                format!("{prefix}/{path}")
            };
            if !index.links.contains(key.as_bytes()) {
                continue;
            }
            let Entry::File(expected_digest) = entry else {
                continue;
            };
            let actual = path
                .split('/')
                .fold(skill.to_path_buf(), |root, part| root.join(part));
            let mut file = super::snapshot_materialized::ordinary_file(&actual)?;
            let mut raw = Vec::new();
            file.by_ref().take(64 * 1024 + 1).read_to_end(&mut raw)?;
            if raw.is_empty() || raw.len() > 64 * 1024 || raw.contains(&0) {
                bail!("Git plaintext link target is empty, oversized or contains NUL");
            }
            if format!("{:x}", Sha256::digest(&raw)) != *expected_digest {
                bail!("Git plaintext link changed during inspection");
            }
            let raw_target = String::from_utf8(raw)?;
            let target = Path::new(&raw_target);
            super::paths::validate_symlink_internal(skill, &actual, target)
                .map_err(|_| anyhow!("Invalid relative Git plaintext link target"))?;
            let target_relative = snapshot_v2::target_key(Path::new(path), target)?;
            *entry = Entry::Link {
                raw_target,
                target_relative,
            };
            plaintext = true;
        }
        versions.current = snapshot_v2::typed_hash(&entries);
        Ok((entries, versions, plaintext))
    }

    pub(super) fn versions(&self, skill: &Path) -> Result<SourceVersions> {
        Ok(self.inventory(skill)?.1)
    }

    pub(super) fn legacy_version(&self, skill: &Path) -> Result<String> {
        let (_repo, repo_id) = super::snapshot_storage::open_directory(&self.original_root)?;
        let (_skill, skill_id) = super::snapshot_storage::open_directory(skill)?;
        let repo = self.original_root.canon()?;
        let resolved = skill.canon()?;
        resolved.strip_prefix(&repo)?;
        let version = super::snapshot::hash_skill_dir(&resolved)?;
        super::snapshot_storage::unchanged(&self.original_root, repo_id)?;
        super::snapshot_storage::unchanged(skill, skill_id)?;
        Ok(version)
    }

    pub(super) fn snapshot(&self, skill: &Path, data: &Path) -> Result<(String, PathBuf)> {
        let (entries, versions, plaintext) = self.inventory(skill)?;
        let content = super::snapshot::snapshot_git_entries(skill, data, &entries, plaintext)?;
        Ok((versions.current, content))
    }
}

fn overlaps(left: &[u8], right: &[u8]) -> bool {
    fn contains(parent: &[u8], child: &[u8]) -> bool {
        parent.is_empty()
            || child == parent
            || child
                .strip_prefix(parent)
                .is_some_and(|tail| tail.starts_with(b"/"))
    }
    contains(left, right) || contains(right, left)
}

fn exact_path(
    root: &Path,
    relative: &str,
    names: &mut BTreeMap<PathBuf, BTreeSet<OsString>>,
    budget: &mut usize,
    deadline: std::time::Instant,
) -> Result<()> {
    let mut parent = root.to_path_buf();
    for part in relative.split('/').filter(|part| !part.is_empty()) {
        if std::time::Instant::now() >= deadline {
            bail!("Git path spelling inspection exceeded its deadline");
        }
        if !matches!(Path::new(part).components().collect::<Vec<_>>().as_slice(), [std::path::Component::Normal(name)] if *name == std::ffi::OsStr::new(part))
        {
            bail!("Git path cannot be represented as one native component");
        }
        if !names.contains_key(&parent) {
            *budget = budget
                .checked_add(parent.as_os_str().as_encoded_bytes().len() + 64)
                .ok_or_else(|| anyhow!("Git path inspection budget overflow"))?;
            if *budget > 4 * 1024 * 1024 {
                bail!("Git path spelling inspection exceeds the 4 MiB limit");
            }
            let (_handle, id) = super::snapshot_storage::open_directory(&parent)?;
            let mut children = BTreeSet::new();
            for child in std::fs::read_dir(&parent)? {
                if std::time::Instant::now() >= deadline {
                    bail!("Git path spelling inspection exceeded its deadline");
                }
                let name = child?.file_name();
                *budget = budget
                    .checked_add(name.as_encoded_bytes().len() + 64)
                    .ok_or_else(|| anyhow!("Git path inspection budget overflow"))?;
                if *budget > 4 * 1024 * 1024 {
                    bail!("Git path spelling inspection exceeds the 4 MiB limit");
                }
                children.insert(name);
            }
            super::snapshot_storage::unchanged(&parent, id)?;
            names.insert(parent.clone(), children);
        }
        let next = parent.join(part);
        if !names[&parent].contains(std::ffi::OsStr::new(part)) {
            return match std::fs::symlink_metadata(&next) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
                Ok(_) => Err(anyhow!("Git index and filesystem path spelling differ")),
            };
        }
        parent = next;
    }
    Ok(())
}
