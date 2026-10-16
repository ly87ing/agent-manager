//! Windows file-link materialization, authenticated by the logical typed v2 ID.
use anyhow::{bail, Context as _, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::snapshot_v2::{typed_hash, Entries, Entry};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileLink {
    pub(super) path: String,
    pub(super) raw_target: String,
}

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

fn key(path: &str) -> Result<()> {
    for part in path.split('/') {
        component(part)?;
    }
    Ok(())
}

fn directory(parts: &[String], entries: &Entries) -> Result<()> {
    if !parts.is_empty() && !matches!(entries.get(&parts.join("/")), Some(Entry::Directory)) {
        bail!("Materialized link traversal requires an included ordinary directory");
    }
    Ok(())
}

fn target(path: &str, raw: &str, entries: &Entries, aliases: &BTreeSet<&str>) -> Result<String> {
    key(path)?;
    let mut parts: Vec<String> = path.split('/').map(str::to_string).collect();
    parts.pop();
    for step in raw.split(['/', '\\']) {
        directory(&parts, entries)?;
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
        || !matches!(entries.get(&resolved), Some(Entry::File(_)))
    {
        bail!("Materialized link target must be an included ordinary non-alias file");
    }
    Ok(resolved)
}

fn validate_keys(entries: &Entries) -> Result<()> {
    for path in entries.keys() {
        key(path)?;
    }
    Ok(())
}

pub(super) fn plan(entries: &Entries) -> Result<Option<Vec<FileLink>>> {
    let links: Vec<_> = entries
        .iter()
        .filter_map(|(path, entry)| {
            if let Entry::Link {
                raw_target,
                target_relative,
            } = entry
            {
                Some((path, raw_target, target_relative))
            } else {
                None
            }
        })
        .collect();
    if links.is_empty()
        || links
            .iter()
            .any(|(_, _, resolved)| !matches!(entries.get(*resolved), Some(Entry::File(_))))
    {
        return Ok(None);
    }
    validate_keys(entries)?;
    let aliases = links.iter().map(|(path, _, _)| path.as_str()).collect();
    let mut records = Vec::new();
    for (path, raw, expected) in links {
        if target(path, raw, entries, &aliases)? != *expected {
            bail!("Windows materialization target differs from its typed source");
        }
        records.push(FileLink {
            path: path.clone(),
            raw_target: raw.clone(),
        });
    }
    Ok(Some(records))
}

pub(super) fn verify(mut entries: Entries, links: &[FileLink]) -> Result<String> {
    validate_keys(&entries)?;
    if links.is_empty()
        || entries
            .values()
            .any(|entry| matches!(entry, Entry::Link { .. }))
    {
        bail!("Materialized snapshot requires file-link records and ordinary physical entries");
    }
    let mut aliases = BTreeSet::new();
    for link in links {
        key(&link.path)?;
        if !aliases.insert(link.path.as_str()) {
            bail!("Duplicate materialized file-link path");
        }
    }
    let mut replacements = Vec::new();
    for link in links {
        let resolved = target(&link.path, &link.raw_target, &entries, &aliases)?;
        match (entries.get(&link.path), entries.get(&resolved)) {
            (Some(Entry::File(alias)), Some(Entry::File(source))) if alias == source => {}
            _ => bail!("Materialized alias bytes differ from their ordinary target"),
        }
        replacements.push((
            link.path.clone(),
            Entry::Link {
                raw_target: link.raw_target.clone(),
                target_relative: resolved,
            },
        ));
    }
    entries.extend(replacements);
    Ok(typed_hash(&entries))
}

pub(super) fn ordinary_file(path: &Path) -> Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            bail!("Materialized content must be an ordinary file");
        }
    }
    if !metadata.is_file() {
        bail!("Materialized content must be an ordinary file");
    }
    Ok(file)
}

fn join_key(root: &Path, key: &str) -> PathBuf {
    key.split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

pub(super) fn copy(
    source: &Path,
    destination: &Path,
    entries: &Entries,
    links: &[FileLink],
) -> Result<()> {
    validate_keys(entries)?;
    let aliases = links.iter().map(|link| link.path.as_str()).collect();
    for (path, entry) in entries {
        let output = join_key(destination, path);
        let input = match entry {
            Entry::Directory => {
                std::fs::create_dir(&output)?;
                continue;
            }
            Entry::File(_) => path.clone(),
            Entry::Link { raw_target, .. } => target(path, raw_target, entries, &aliases)?,
        };
        let mut input = ordinary_file(&join_key(source, &input))?;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)?;
        std::io::copy(&mut input, &mut output).context("Copying materialized skill bytes")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            output.set_permissions(std::fs::Permissions::from_mode(0o444))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Entries {
        [
            ("alias", Entry::File("payload-digest".into())),
            ("nested", Entry::Directory),
            ("nested/alias", Entry::File("payload-digest".into())),
            ("nested/empty", Entry::Directory),
            ("z-target", Entry::File("payload-digest".into())),
        ]
        .into_iter()
        .map(|(path, entry)| (path.into(), entry))
        .collect()
    }

    #[test]
    fn windows_relative_traversal_is_exact_on_every_verifier_host() {
        let entries = entries();
        let aliases = ["alias", "nested/alias"].into_iter().collect();
        for raw in ["../z-target", r"..\z-target", "./empty/../../z-target"] {
            assert_eq!(
                target("nested/alias", raw, &entries, &aliases).unwrap(),
                "z-target"
            );
        }
        for raw in [
            "",
            "/z-target",
            r"\z-target",
            "//z-target",
            "./z-target/",
            "missing/../z-target",
            "z-target/../z-target",
            "alias/../z-target",
            "nested/alias/../../z-target",
            "../z-target",
            "nested/empty",
            "Z-TARGET",
            "Z-TARG~1",
            "C:z-target",
            "C:/z-target",
            r"\\?\C:\z-target",
            r"\??\C:\z-target",
            "nested//../z-target",
            "z-target:stream",
            "CONIN$",
            "conout$.txt",
            "NUL .txt",
            "COM¹",
            "LPT².txt",
        ] {
            assert!(target("alias", raw, &entries, &aliases).is_err(), "{raw:?}");
        }
        for path in [
            "",
            ".",
            "..",
            "/alias",
            "a//alias",
            "a/../alias",
            "a\\alias",
            "alias.",
            "alias ",
            "a\0b",
            "a\u{1f}b",
            "a:b",
            "a?b",
            "a*b",
            "a<b",
            "a>b",
            "a|b",
            "a\"b",
            "con",
            "PRN.txt",
            "AUX .txt",
            "COM9.log",
            "LPT³",
            "CONIN$.log",
            "conout$",
        ] {
            assert!(key(path).is_err(), "{path:?}");
        }
        for path in ["nested/alias", "COM10", "LPT0", "context", "名字.txt"] {
            key(path).unwrap();
        }
        let chain = vec![
            FileLink {
                path: "alias".into(),
                raw_target: "nested/alias".into(),
            },
            FileLink {
                path: "nested/alias".into(),
                raw_target: "../alias".into(),
            },
        ];
        assert!(verify(entries, &chain).is_err());
    }

    #[test]
    fn new_representation_only_covers_direct_included_file_links() {
        let mut entries = entries();
        assert!(plan(&entries).unwrap().is_none());
        for resolved in ["nested", "missing", "nested/alias"] {
            entries.insert(
                "alias".into(),
                Entry::Link {
                    raw_target: resolved.into(),
                    target_relative: resolved.into(),
                },
            );
            if resolved == "nested/alias" {
                entries.insert(
                    "nested/alias".into(),
                    Entry::Link {
                        raw_target: "../z-target".into(),
                        target_relative: "z-target".into(),
                    },
                );
            }
            assert!(plan(&entries).unwrap().is_none(), "{resolved}");
        }
    }

    #[test]
    fn writer_copies_later_targets_and_nested_links_without_creating_file_links() {
        let root = std::env::temp_dir().join(format!(
            "am-materialized-writer-{}",
            crate::store::uuid_v4()
        ));
        let source = root.join("source");
        let output = root.join("output");
        std::fs::create_dir_all(source.join("nested/empty")).unwrap();
        std::fs::create_dir(&output).unwrap();
        std::fs::write(source.join("SKILL.md"), b"# Fixture\n").unwrap();
        std::fs::write(source.join("z-target"), b"default stream payload").unwrap();
        #[cfg(unix)]
        for (path, raw) in [
            ("alias", "./z-target"),
            ("nested/alias", "./empty/../../z-target"),
        ] {
            std::os::unix::fs::symlink(raw, source.join(path)).unwrap();
        }
        #[cfg(windows)]
        for (path, raw) in [
            ("alias", r".\z-target"),
            ("nested/alias", r".\empty\..\..\z-target"),
        ] {
            std::os::windows::fs::symlink_file(raw, source.join(path)).unwrap();
        }
        let entries = super::super::snapshot_v2::source_entries(&source).unwrap();
        let version = typed_hash(&entries);
        #[cfg(windows)]
        std::fs::write(
            source.join("z-target:materialization-fixture"),
            b"unhashed stream",
        )
        .unwrap();
        let source = std::fs::canonicalize(source).unwrap();
        let output = std::fs::canonicalize(output).unwrap();
        let links = plan(&entries).unwrap().unwrap();
        copy(&source, &output, &entries, &links).unwrap();
        let physical = super::super::snapshot_v2::physical_entries(&output).unwrap();
        assert_eq!(verify(physical, &links).unwrap(), version);
        for path in ["alias", "nested/alias"] {
            let metadata = std::fs::symlink_metadata(join_key(&output, path)).unwrap();
            assert!(metadata.is_file() && !metadata.file_type().is_symlink());
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                assert_eq!(
                    metadata.file_attributes()
                        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT,
                    0
                );
            }
            assert_eq!(
                std::fs::read(join_key(&output, path)).unwrap(),
                b"default stream payload"
            );
            #[cfg(windows)]
            assert!(std::fs::read(join_key(
                &output,
                &format!("{path}:materialization-fixture")
            ))
            .is_err());
        }
        #[cfg(windows)]
        assert!(std::fs::read(output.join("z-target:materialization-fixture")).is_err());
        assert!(
            copy(&source, &output, &entries, &links).is_err(),
            "Existing output files must never be overwritten"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
