//! A successful uv command still needs independent installed-version evidence.
use anyhow::{Context as _, Result};
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
