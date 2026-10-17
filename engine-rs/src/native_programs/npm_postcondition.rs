//! Read actual npm identity and declared entry evidence before publication.
use super::uv_metadata_files::Directory;
use super::uv_postcondition::{UnverifiedEntry, UnverifiedVersion};
use crate::platform::Canon;
use anyhow::Result;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

pub(super) fn exact_version(value: &str) -> Option<String> {
    value
        .strip_prefix('v')
        .unwrap_or(value)
        .parse::<semver::Version>()
        .ok()
        .map(|v| v.to_string())
}

#[derive(Default)]
struct Manifest {
    name: Option<String>,
    version: Option<String>,
    bin: Option<Box<RawValue>>,
}

impl<'de> Deserialize<'de> for Manifest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Fields;
        impl<'de> Visitor<'de> for Fields {
            type Value = Manifest;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an npm manifest")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Manifest, M::Error> {
                let mut manifest = Manifest::default();
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "name" if manifest.name.is_none() => {
                            manifest.name = Some(map.next_value()?)
                        }
                        "version" if manifest.version.is_none() => {
                            manifest.version = Some(map.next_value()?)
                        }
                        "bin" if manifest.bin.is_none() => manifest.bin = Some(map.next_value()?),
                        "name" | "version" | "bin" => {
                            return Err(de::Error::custom("duplicate npm identity/bin field"))
                        }
                        _ => {
                            map.next_value::<de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(manifest)
            }
        }
        deserializer.deserialize_map(Fields)
    }
}

struct Bins(Vec<(String, String)>);
impl<'de> Deserialize<'de> for Bins {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Fields;
        impl<'de> Visitor<'de> for Fields {
            type Value = Bins;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("npm bin mappings")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Bins, E> {
                Ok(Bins(vec![(String::new(), value.into())]))
            }
            fn visit_seq<S: SeqAccess<'de>>(
                self,
                mut seq: S,
            ) -> std::result::Result<Bins, S::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<String>()? {
                    let name = value
                        .rsplit(['/', '\\'])
                        .find(|part| !part.is_empty())
                        .unwrap_or_default()
                        .to_string();
                    values.push((name, value));
                }
                Ok(Bins(values))
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Bins, M::Error> {
                let mut values = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, String>()? {
                    if values.insert(key, value).is_some() {
                        return Err(de::Error::custom("duplicate npm bin"));
                    }
                }
                Ok(Bins(values.into_iter().collect()))
            }
        }
        deserializer.deserialize_any(Fields)
    }
}

// npm-normalize-package-bin anchors targets at '/', then removes that root.
fn normalized_target(value: &str) -> Option<PathBuf> {
    if value.chars().any(char::is_control) {
        return None;
    }
    let mut parts = Vec::new();
    for part in value.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.iter().collect())
}

fn mappings(manifest: &Manifest) -> Option<BTreeMap<String, PathBuf>> {
    let raw = manifest.bin.as_ref()?;
    let Bins(mut bins) = serde_json::from_str(raw.get()).ok()?;
    if raw.get().trim_start().starts_with('"') {
        bins[0].0 = manifest.name.clone()?;
    }
    let mut output = BTreeMap::new();
    for (key, value) in bins {
        let clean = key.replace(['\\', ':'], "/");
        let key = clean.rsplit('/').find(|part| !part.is_empty())?;
        if matches!(key, "." | "..")
            || !super::declare::valid_bin(key)
            || output
                .insert(key.into(), normalized_target(&value)?)
                .is_some()
        {
            return None;
        }
    }
    (!output.is_empty()).then_some(output)
}

fn bytes(mut file: std::fs::File, max: u64) -> Option<Vec<u8>> {
    if file.metadata().ok()?.len() > max {
        return None;
    }
    let mut bytes = Vec::new();
    (&mut file).take(max + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= max).then_some(bytes)
}

fn package(directory: &Path, name: &str) -> Option<(Directory, PathBuf)> {
    let root = directory
        .parent()?
        .canon()
        .ok()?
        .join(directory.file_name()?);
    let root_guard = Directory::open(&root).ok()?;
    let mut guard = root_guard.child("node_modules".as_ref()).ok()?;
    let mut path = root.join("node_modules");
    for part in name.split('/') {
        guard = guard.child(part.as_ref()).ok()?;
        path.push(part);
    }
    Some((guard, path))
}

fn display(path: &Path, home: &Path) -> String {
    path.strip_prefix(home)
        .map(|rest| format!("~/{}", crate::platform::slash(rest)))
        .unwrap_or_else(|_| path.display().to_string())
}

pub(super) fn absent(path: &Path, home: &Path) -> Result<()> {
    if matches!(std::fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
    {
        return Ok(());
    }
    Err(UnverifiedVersion(format!("An existing incomplete npm candidate or staging object at {} is retained. Inspect and move it explicitly before retrying; it will not be deleted or overwritten automatically.", display(path, home))).into())
}

#[cfg(windows)]
fn shim_target(text: &str) -> Option<PathBuf> {
    let invocation = regex::Regex::new(r#""%(?:dp0%|~dp0)\\?([^"%\r\n]+)"[ \t]+%\*$"#).ok()?;
    let mut found = None;
    for line in text.lines().map(str::trim) {
        let line = line.strip_prefix('@').unwrap_or(line);
        let direct = line.starts_with("\"%dp0%") || line.starts_with("\"%~dp0");
        let indirect = line
            .starts_with("endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\" ");
        if !direct && !indirect {
            continue;
        }
        let capture = invocation.captures(line)?;
        if direct && capture.get(0)?.start() != 0 {
            return None;
        }
        let target = PathBuf::from(capture.get(1)?.as_str().replace('\\', "/"));
        if found.as_ref().is_some_and(|previous| previous != &target) {
            return None;
        }
        found = Some(target);
    }
    found
}

pub(super) fn check(
    directory: &Path,
    name: &str,
    target: &str,
    bins: &[String],
    home: &Path,
    stage: &str,
) -> Result<()> {
    let evidence = (|| -> Option<_> {
        let (guard, path) = package(directory, name)?;
        let manifest: Manifest = serde_json::from_slice(&bytes(
            guard.file("package.json".as_ref()).ok()?,
            1024 * 1024,
        )?)
        .ok()?;
        (manifest.name.as_deref() == Some(name) && manifest.version.as_deref() == Some(target))
            .then_some((guard, path, manifest))
    })();
    let Some((_guard, package, manifest)) = evidence else {
        return Err(UnverifiedVersion(format!("Managed npm identity/version verification before {stage} failed. Expected {name}@{target}. This {stage} step was not performed. The candidate at {} is retained without automatic restoration or cleanup after this check. Earlier completion metadata or previously active contents may exist.", display(directory, home))).into());
    };
    let entries = (|| -> Option<()> {
        if bins.is_empty() {
            return None;
        }
        let mappings = mappings(&manifest)?;
        let root = directory
            .parent()?
            .canon()
            .ok()?
            .join(directory.file_name()?);
        let package = package.canon().ok()?;
        for bin in bins {
            let target = package.join(mappings.get(bin)?).canon().ok()?;
            if !target.starts_with(&package) {
                return None;
            }
            let parent = Directory::open(target.parent()?).ok()?;
            let file = parent.file(target.file_name()?).ok()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if file.metadata().ok()?.permissions().mode() & 0o111 == 0 {
                    return None;
                }
                if root.join("node_modules/.bin").join(bin).canon().ok()? != target {
                    return None;
                }
            }
            #[cfg(windows)]
            {
                let _file = file;
                let entry_dir = root.join("node_modules/.bin");
                let guard = Directory::open(&entry_dir).ok()?;
                let text = String::from_utf8(bytes(
                    guard.file(format!("{bin}.cmd").as_ref()).ok()?,
                    64 * 1024,
                )?)
                .ok()?;
                if entry_dir.join(shim_target(&text)?).canon().ok()? != target {
                    return None;
                }
            }
        }
        Some(())
    })();
    if entries.is_some() {
        return Ok(());
    }
    Err(UnverifiedEntry(format!("Managed npm command verification before {stage} failed. Declared bin targets and their generated entries must identify ordinary files inside this package, executable on Unix. This {stage} step was not performed. The candidate at {} is retained without automatic restoration or cleanup after this check. This evidence does not establish runtime/interpreter availability, authenticity or arbitrary program behavior.", display(directory, home))).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn npm_bin_normalization_preserves_native_forms_and_refuses_ambiguity() {
        let parse = |name: &str, bin: &str| {
            let manifest: Manifest = serde_json::from_str(&format!(
                r#"{{"name":"{name}","version":"1.0.0","bin":{bin}}}"#
            ))
            .unwrap();
            mappings(&manifest)
        };
        for bin in [r#""./bin/cli.js""#, r#"{"tool-a":"../bin/cli.js"}"#] {
            assert_eq!(
                parse("@scope/tool-a", bin).unwrap()["tool-a"],
                PathBuf::from("bin/cli.js")
            );
        }
        assert_eq!(
            parse("tool-a", r#"["bin/tool-a"]"#).unwrap()["tool-a"],
            PathBuf::from("bin/tool-a")
        );
        assert_eq!(
            parse("tool-a", r#"{"./tool-a":"bin\\cli.js"}"#).unwrap()["tool-a"],
            PathBuf::from("bin/cli.js")
        );
        for bin in [
            r#"{"tool-a":"a","tool-a":"b"}"#,
            r#"{"./tool-a":"a","tool-a":"b"}"#,
            r#"{"..":"a"}"#,
            r#"{"tool-a":null}"#,
        ] {
            assert!(parse("tool-a", bin).is_none(), "{bin}");
        }
        for text in [
            r#"{"name":"a","name":"b"}"#,
            r#"{"version":"1","version":"2"}"#,
            r#"{"bin":{},"bin":{}}"#,
        ] {
            assert!(serde_json::from_str::<Manifest>(text).is_err());
        }
        assert_eq!(exact_version("v1.2.3"), Some("1.2.3".into()));
        assert!(exact_version("next").is_none());
        assert!(exact_version("1").is_none());
    }
    #[cfg(windows)]
    #[test]
    fn native_shim_invocations_require_targets_not_comments_or_markers() {
        assert_eq!(
            shim_target("@echo off\r\n\"%~dp0..\\tool-a\\bin\\cli.js\" %*\r\n"),
            Some(PathBuf::from("../tool-a/bin/cli.js"))
        );
        assert_eq!(shim_target("endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\..\\tool-a\\bin\\cli.js\" %*"), Some(PathBuf::from("../tool-a/bin/cli.js")));
        assert!(shim_target("REM \"%~dp0..\\tool-a\\bin\\cli.js\" %*").is_none());
        assert!(shim_target(":: AM_CMD_TARGET: C:\\file.js").is_none());
        assert!(shim_target("\"%~dp0a.js\" %*\n\"%~dp0b.js\" %*").is_none());
    }
}
