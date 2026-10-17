//! A successful uv command still needs independent installed-version evidence.
use crate::platform::Canon;
use anyhow::{anyhow, Context as _, Result};
use pep440_rs::{Version, VersionSpecifier};
use std::path::{Path, PathBuf};

pub(super) struct Verification {
    package: PathBuf,
    name: String,
    target: String,
    expected: VersionSpecifier,
    backup: PathBuf,
}

#[derive(Debug)]
pub(super) struct UnverifiedVersion(String);

impl std::fmt::Display for UnverifiedVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for UnverifiedVersion {}

fn display(path: &Path, home: &Path) -> String {
    path.strip_prefix(home)
        .map(|rest| format!("~/{}", crate::platform::slash(rest)))
        .unwrap_or_else(|_| path.display().to_string())
}

pub(super) fn managed_target(name: &str, target: &str) -> Result<(String, VersionSpecifier)> {
    let name = super::uv_metadata::normalized_name(name)
        .ok_or_else(|| anyhow!("Invalid PyPI package name"))?;
    let version: Version = target.parse().context("Invalid PyPI target version")?;
    Ok((name, VersionSpecifier::equals_version(version)))
}

pub(super) fn incomplete_candidate(directory: &Path, home: &Path) -> anyhow::Error {
    UnverifiedVersion(format!(
        "Incomplete PyPI candidate already exists at {}. Inspect and move it explicitly before retrying; it will not be overwritten automatically.",
        display(directory, home),
    ))
    .into()
}

pub(super) fn check_managed(
    directory: &Path,
    name: &str,
    target: &str,
    home: &Path,
    stage: &str,
) -> Result<()> {
    let (normalized, expected) = managed_target(name, target)?;
    let actual = directory
        .parent()
        .and_then(|parent| parent.canon().ok())
        .and_then(|parent| {
            let directory = parent.join(directory.file_name()?);
            super::uv_metadata::version(&directory.join("tools").join(normalized), name).ok()
        });
    if actual
        .as_deref()
        .and_then(|text| text.parse::<Version>().ok())
        .is_some_and(|version| expected.contains(&version))
    {
        return Ok(());
    }
    Err(UnverifiedVersion(format!(
        "Managed PyPI version verification before {stage} failed: installed version is {}; requested {target}. \
         This {stage} step was not performed. Earlier completion metadata or previously active contents may still exist. \
         The candidate at {} is retained without automatic restoration or cleanup after this check. \
         Inspect the actual files before any further action.",
        actual.as_deref().unwrap_or("unknown"),
        display(directory, home),
    ))
    .into())
}

impl Verification {
    pub(super) fn new(package: &Path, name: &str, target: &str, backup: &Path) -> Result<Self> {
        let version: Version = target.parse().context("Invalid PyPI target version")?;
        Ok(Self {
            package: package.to_path_buf(),
            name: name.into(),
            target: target.into(),
            expected: VersionSpecifier::equals_version(version),
            backup: backup.to_path_buf(),
        })
    }

    pub(super) fn check(&self, home: &Path, keep_backups: usize) -> Result<()> {
        let actual = super::uv_metadata::version(&self.package, &self.name).ok();
        if actual
            .as_deref()
            .and_then(|text| text.parse::<Version>().ok())
            .is_some_and(|version| self.expected.contains(&version))
        {
            return Ok(());
        }
        Err(UnverifiedVersion(format!(
            "uv finished, but the installed version is {}; requested {}. \
             The current install at {} is unverified and may remain on PATH. \
             No automatic restoration or cleanup of the current package or backup was attempted after this check. \
             Inspect the current location and backup {}. Retained backups remain \
             subject to the normal newest-{keep_backups} limit after any successful \
             action for this program.",
            actual.as_deref().unwrap_or("unknown"),
            self.target,
            display(&self.package, home),
            display(&self.backup, home),
        ))
        .into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_evidence_accepts_parent_aliases_but_refuses_a_linked_version_root() {
        let root = std::env::temp_dir().join(format!("am-managed-uv-roots-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canon().unwrap();
        let programs = root.join("programs");
        let version = programs.join("3.0");
        let metadata = version.join("tools/tool-a").join(if cfg!(windows) {
            "Lib/site-packages/tool_a-3.0.dist-info/METADATA"
        } else {
            "lib/python3.13/site-packages/tool_a-3.0.dist-info/METADATA"
        });
        std::fs::create_dir_all(metadata.parent().unwrap()).unwrap();
        std::fs::write(
            &metadata,
            "Metadata-Version: 2.4\nName: tool-a\nVersion: 3.0\n\n",
        )
        .unwrap();
        let alias = root.join("parent-alias");
        let linked = programs.join("linked-version");
        crate::platform::link_dir(&programs, &alias).unwrap();
        crate::platform::link_dir(&version, &linked).unwrap();
        check_managed(&alias.join("3.0"), "tool-a", "3.0", &root, "completion").unwrap();
        assert!(check_managed(&linked, "tool-a", "3.0", &root, "completion")
            .unwrap_err()
            .is::<UnverifiedVersion>());
        crate::platform::remove_link(&alias).unwrap();
        crate::platform::remove_link(&linked).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
