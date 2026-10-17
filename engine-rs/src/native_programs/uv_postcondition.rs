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
pub(super) struct UnverifiedVersion(pub(super) String);

impl std::fmt::Display for UnverifiedVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for UnverifiedVersion {}

#[derive(Debug)]
pub(super) struct UnverifiedEntry(pub(super) String);

impl std::fmt::Display for UnverifiedEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for UnverifiedEntry {}

pub(super) fn check_entries(
    directory: &Path,
    entries: &[PathBuf],
    home: &Path,
    stage: &str,
) -> Result<()> {
    use super::uv_metadata_files::Directory;
    let verified = (|| -> Option<()> {
        if entries.is_empty() {
            return None;
        }
        let root = directory
            .parent()?
            .canon()
            .ok()?
            .join(directory.file_name()?);
        let _root_guard = Directory::open(&root).ok()?;
        let root = root.canon().ok()?;
        for entry in entries {
            let actual = entry.canon().ok()?;
            if !actual.starts_with(&root) {
                return None;
            }
            let parent = Directory::open(actual.parent()?).ok()?;
            let file = parent.file(actual.file_name()?).ok()?;
            let metadata = file.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 == 0 {
                    return None;
                }
            }
        }
        Some(())
    })()
    .is_some();
    if verified {
        return Ok(());
    }
    Err(UnverifiedEntry(format!(
        "Managed PyPI command verification before {stage} failed. Declared entries must resolve to ordinary command files inside this candidate, with executable permissions on Unix. \
         This {stage} step was not performed. Earlier completion metadata or previously active contents may still exist. \
         The candidate at {} is retained without automatic restoration or cleanup after this check. Inspect its bin entries and actual files before any further action.",
        display(directory, home),
    )).into())
}

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
    fn managed_entries_require_internal_ordinary_executable_targets() {
        let root =
            std::env::temp_dir().join(format!("am-managed-entries-{}", crate::store::uuid_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canon().unwrap();
        let version = root.join("programs/1.0");
        let scripts = version.join("tools/tool-a");
        let bin = version.join("bin");
        std::fs::create_dir_all(&scripts).unwrap();
        std::fs::create_dir(&bin).unwrap();
        let target = scripts.join("tool-a");
        std::fs::write(&target, b"private fixture bytes").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let entry = bin.join(if cfg!(windows) {
            "tool-a.exe"
        } else {
            "tool-a"
        });
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &entry).unwrap();
        #[cfg(windows)]
        std::fs::copy(&target, &entry).unwrap();
        check_entries(&version, &[entry.clone()], &root, "completion").unwrap();
        assert!(check_entries(&version, &[], &root, "completion")
            .unwrap_err()
            .is::<UnverifiedEntry>());
        let alias = root.join("parent-alias");
        crate::platform::link_dir(&root.join("programs"), &alias).unwrap();
        check_entries(
            &alias.join("1.0"),
            &[alias.join("1.0/bin").join(entry.file_name().unwrap())],
            &root,
            "selection",
        )
        .unwrap();
        crate::platform::remove_link(&alias).unwrap();
        std::fs::remove_file(&entry).unwrap();
        assert!(
            check_entries(&version, &[entry.clone()], &root, "selection")
                .unwrap_err()
                .is::<UnverifiedEntry>()
        );
        #[cfg(unix)]
        std::os::unix::fs::symlink(std::env::current_exe().unwrap(), &entry).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(std::env::current_exe().unwrap(), &entry).unwrap();
        assert!(check_entries(&version, &[entry.clone()], &root, "selection").is_err());
        std::fs::remove_file(&entry).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::copy(&target, &entry).unwrap();
            std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(check_entries(&version, &[entry.clone()], &root, "selection").is_err());
            std::fs::remove_file(&entry).unwrap();
        }
        std::fs::create_dir(&entry).unwrap();
        assert!(check_entries(&version, &[entry], &root, "selection").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

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
