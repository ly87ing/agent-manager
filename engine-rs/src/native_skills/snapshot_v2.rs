//! Typed Git snapshot fingerprints; the legacy hash remains a separate contract.
use anyhow::{bail, Context as _, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Component, Path};

use super::paths::validate_symlink_internal;

const MANIFEST: &str = "snapshot.json";

pub(super) struct SourceVersions {
    pub(super) legacy: String,
    pub(super) current: String,
}

pub(super) enum Entry {
    File(String),
    Directory,
    Link {
        raw_target: String,
        target_relative: String,
    },
}

pub(super) type Entries = BTreeMap<String, Entry>;

fn relative_key(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for part in path.components() {
        match part {
            Component::Normal(name) => parts.push(
                name.to_str()
                    .context("Skill paths must be UTF-8 for typed snapshots")?,
            ),
            _ => bail!("Typed snapshot path must be relative and normalized"),
        }
    }
    Ok(parts.join("/"))
}

fn target_key(relative_link: &Path, target: &Path) -> Result<String> {
    let mut resolved = relative_link
        .parent()
        .unwrap_or(Path::new(""))
        .to_path_buf();
    for part in target.components() {
        match part {
            Component::Normal(name) => resolved.push(name),
            Component::CurDir => {}
            Component::ParentDir if resolved.pop() => {}
            _ => bail!("Typed snapshot link target escapes its root"),
        }
    }
    relative_key(&resolved)
}

pub(super) fn source_versions(root: &Path) -> Result<SourceVersions> {
    let entries = source_entries(root)?;
    let legacy = entries
        .iter()
        .filter_map(|(path, entry)| {
            let digest = match entry {
                Entry::File(digest) => digest.clone(),
                Entry::Directory => return None,
                Entry::Link { raw_target, .. } => {
                    let mut hash = Sha256::new();
                    hash.update(b"symlink:");
                    hash.update(raw_target.as_bytes());
                    format!("{:x}", hash.finalize())
                }
            };
            Some((path.replace('\\', "/"), digest))
        })
        .collect();
    Ok(SourceVersions {
        legacy: super::snapshot::hash_entries(legacy),
        current: typed_hash(&entries),
    })
}

pub(super) fn source_entries(root: &Path) -> Result<Entries> {
    collect_entries(root, false)
}

pub(super) fn physical_entries(root: &Path) -> Result<Entries> {
    collect_entries(root, true)
}

fn collect_entries(root: &Path, materialized: bool) -> Result<Entries> {
    let mut entries = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    let mut buffer = [0u8; 64 * 1024];
    while let Some(directory) = stack.pop() {
        for child in std::fs::read_dir(&directory)? {
            let path = child?.path();
            let metadata = std::fs::symlink_metadata(&path)?;
            if !materialized
                && metadata.is_dir()
                && super::EXCLUDED_DIRS
                    .contains(&path.file_name().unwrap().to_string_lossy().as_ref())
            {
                continue;
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes()
                    & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                    != 0
                    && !metadata.file_type().is_symlink()
                {
                    bail!("Unsupported reparse point in typed skill snapshot");
                }
            }
            let relative = path.strip_prefix(root)?;
            let key = relative_key(relative)?;
            let entry = if metadata.file_type().is_symlink() {
                if materialized {
                    bail!("Materialized snapshot contains a physical symbolic link");
                }
                let raw = std::fs::read_link(&path)?;
                validate_symlink_internal(root, &path, &raw)?;
                let raw_text = raw
                    .to_str()
                    .context("Skill link targets must be UTF-8 for typed snapshots")?;
                Entry::Link {
                    raw_target: raw_text.into(),
                    target_relative: target_key(relative, &raw)?,
                }
            } else if metadata.is_dir() {
                stack.push(path.clone());
                Entry::Directory
            } else if metadata.is_file() {
                let mut file = if materialized {
                    super::snapshot_materialized::ordinary_file(&path)?
                } else {
                    std::fs::File::open(&path)?
                };
                let mut hash = Sha256::new();
                loop {
                    let count = match file.read(&mut buffer) {
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        result => result?,
                    };
                    if count == 0 {
                        break;
                    }
                    hash.update(&buffer[..count]);
                }
                Entry::File(format!("{:x}", hash.finalize()))
            } else {
                bail!("Unsupported entry in typed skill snapshot");
            };
            if entries.insert(key, entry).is_some() {
                bail!("Duplicate path in typed skill snapshot");
            }
        }
    }
    Ok(entries)
}

pub(super) fn typed_hash(entries: &Entries) -> String {
    let mut hash = Sha256::new();
    hash.update(b"\0agent-manager-skill-v2\0");
    for (path, entry) in entries {
        let (kind, digest, target) = match entry {
            Entry::File(digest) => (b'f', digest.clone(), ""),
            Entry::Directory => (b'd', String::new(), ""),
            Entry::Link {
                raw_target,
                target_relative,
            } => (
                b'l',
                format!("{:x}", Sha256::digest(raw_target.as_bytes())),
                target_relative.as_str(),
            ),
        };
        hash.update([kind, 0]);
        for value in [path.as_str(), digest.as_str(), target] {
            hash.update(value.as_bytes());
            hash.update([0]);
        }
    }
    format!("{:x}", hash.finalize())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    version: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MaterializedManifest {
    schema: u32,
    version: String,
    materialized_file_links: Vec<super::snapshot_materialized::FileLink>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SnapshotManifest {
    Typed(Manifest),
    Materialized(MaterializedManifest),
}

pub(super) fn write_manifest(version_dir: &Path, version: &str) -> Result<()> {
    let bytes = serde_json::to_vec(&Manifest {
        schema: 2,
        version: version.into(),
    })?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(version_dir.join(MANIFEST))?
        .write_all(&bytes)?;
    Ok(())
}

pub(super) fn write_materialized_manifest(
    version_dir: &Path,
    version: &str,
    links: &[super::snapshot_materialized::FileLink],
) -> Result<()> {
    let bytes = serde_json::to_vec(&MaterializedManifest {
        schema: 3,
        version: version.into(),
        materialized_file_links: links.to_vec(),
    })?;
    if bytes.len() > 64 * 1024 {
        bail!("Materialized snapshot manifest exceeds 64 KiB");
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(version_dir.join(MANIFEST))?
        .write_all(&bytes)?;
    Ok(())
}

/// Only an absent manifest selects the legacy format; invalid metadata never does.
pub(super) fn snapshot_hash(
    version_dir: &Path,
    content: &Path,
    version: &str,
) -> Result<Option<String>> {
    let path = version_dir.join(MANIFEST);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
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
    let file = options.open(&path)?;
    let metadata = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            bail!("Snapshot manifest must be an ordinary file");
        }
    }
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        bail!("Snapshot manifest must be an ordinary file of at most 64 KiB");
    }
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 {
        bail!("Snapshot manifest exceeds 64 KiB");
    }
    match serde_json::from_slice::<SnapshotManifest>(&bytes)? {
        SnapshotManifest::Typed(manifest) => {
            if manifest.schema != 2 || manifest.version != version || bytes.len() > 4096 {
                bail!("Snapshot manifest schema, version or size does not match");
            }
            Ok(Some(source_versions(content)?.current))
        }
        SnapshotManifest::Materialized(manifest) => {
            if manifest.schema != 3 || manifest.version != version {
                bail!("Materialized snapshot schema or version does not match");
            }
            Ok(Some(super::snapshot_materialized::verify(
                physical_entries(content)?,
                &manifest.materialized_file_links,
            )?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::snapshot::{
        hash_skill_dir, snapshot_git_skill, snapshot_skill, verify_skill_version,
    };
    use super::*;
    use std::path::PathBuf;

    struct Root(PathBuf);
    impl Root {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("am-typed-snapshot-{}", crate::store::uuid_v4()));
            std::fs::create_dir_all(path.join("source")).unwrap();
            std::fs::write(path.join("source/SKILL.md"), b"# Fixture\n").unwrap();
            Self(path)
        }
        fn source(&self) -> PathBuf {
            self.0.join("source")
        }
        fn data(&self) -> PathBuf {
            self.0.join("data")
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            #[cfg(windows)]
            let _ = super::super::fork::make_writable(&self.0);
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn materialized_snapshot_verifies_typed_links_from_actual_file_bytes() {
        let root = Root::new();
        let source = root.source();
        std::fs::write(source.join("z-target"), b"payload").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("./z-target", source.join("alias")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(r".\z-target", source.join("alias")).unwrap();
        let raw_target = std::fs::read_link(source.join("alias"))
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let version = source_versions(&source).unwrap().current;
        let version_dir = root.data().join("skill-versions").join(&version);
        let content = version_dir.join("content");
        std::fs::create_dir_all(&content).unwrap();
        for name in ["SKILL.md", "z-target", "alias"] {
            std::fs::write(
                content.join(name),
                std::fs::read(source.join(name)).unwrap(),
            )
            .unwrap();
        }
        let manifest = serde_json::json!({
            "schema": 3, "version": version,
            "materialized_file_links": [{"path": "alias", "raw_target": raw_target}]
        });
        std::fs::write(version_dir.join(MANIFEST), manifest.to_string()).unwrap();
        assert_eq!(
            verify_skill_version(&root.data(), &version).unwrap(),
            content
        );
        for name in ["alias", "z-target"] {
            std::fs::write(content.join(name), b"same new payload").unwrap();
        }
        assert!(
            verify_skill_version(&root.data(), &version).is_err(),
            "Matching alias/target bytes still require the fixed logical ID"
        );
        for name in ["alias", "z-target"] {
            std::fs::write(content.join(name), b"payload").unwrap();
        }
        let padded = format!("{manifest}{}", " ".repeat(4096));
        assert!(padded.len() > 4096 && padded.len() < 64 * 1024);
        std::fs::write(version_dir.join(MANIFEST), padded).unwrap();
        assert_eq!(
            verify_skill_version(&root.data(), &version).unwrap(),
            content
        );
        for name in ["alias", "z-target"] {
            std::fs::write(content.join(name), b"changed").unwrap();
            assert!(verify_skill_version(&root.data(), &version).is_err());
            std::fs::write(content.join(name), b"payload").unwrap();
        }
        for name in super::super::EXCLUDED_DIRS {
            std::fs::create_dir(content.join(name)).unwrap();
            assert!(
                verify_skill_version(&root.data(), &version).is_err(),
                "{name}"
            );
            std::fs::write(content.join(name).join("extra"), b"unbound").unwrap();
            assert!(
                verify_skill_version(&root.data(), &version).is_err(),
                "{name}"
            );
            std::fs::remove_file(content.join(name).join("extra")).unwrap();
            std::fs::remove_dir(content.join(name)).unwrap();
        }
        let external = root.0.join("external-directory");
        std::fs::create_dir(&external).unwrap();
        std::fs::write(external.join("marker"), b"external").unwrap();
        crate::platform::link_dir(&external, &content.join(".git")).unwrap();
        assert!(verify_skill_version(&root.data(), &version).is_err());
        assert_eq!(std::fs::read(external.join("marker")).unwrap(), b"external");
        crate::platform::remove_link(content.join(".git")).unwrap();
        let record = serde_json::json!({"path": "alias", "raw_target": raw_target});
        let mut invalid = vec![
            serde_json::json!({"schema": 3, "version": version}).to_string(),
            serde_json::json!({"schema": 3, "version": version, "materialized_file_links": null}).to_string(),
            serde_json::json!({"schema": 4, "version": version, "materialized_file_links": [record]}).to_string(),
            serde_json::json!({"schema": 3, "version": version, "materialized_file_links": [record], "digest": "untrusted"}).to_string(),
            serde_json::json!({"schema": 3, "version": version, "materialized_file_links": []}).to_string(),
            serde_json::json!({"schema": 3, "version": version, "materialized_file_links": [record, record]}).to_string(),
            serde_json::json!({"schema": 3, "version": version, "materialized_file_links": [{"path": "alias", "raw_target": "./z-target", "digest": "untrusted"}]}).to_string(),
            format!("{{\"schema\":3,\"schema\":3,\"version\":\"{version}\",\"materialized_file_links\":[{record}]}}"),
            format!("{{\"schema\":3,\"version\":\"{version}\",\"materialized_file_links\":[{{\"path\":\"alias\",\"path\":\"alias\",\"raw_target\":\"./z-target\"}}]}}"),
            format!("{{\"schema\":2,\"version\":\"{version}\",\"materialized_file_links\":null}}"),
            " ".repeat(64 * 1024 + 1),
        ];
        for raw in [
            "alias",
            "missing/../z-target",
            "z-target/../z-target",
            "Z-TARGET",
            "../z-target",
            "C:z-target",
            r"\\?\C:\z-target",
            "NUL.txt",
            "z-target:stream",
        ] {
            invalid.push(serde_json::json!({"schema": 3, "version": version, "materialized_file_links": [{"path": "alias", "raw_target": raw}]}).to_string());
        }
        for bytes in invalid {
            std::fs::write(version_dir.join(MANIFEST), bytes).unwrap();
            assert!(verify_skill_version(&root.data(), &version).is_err());
        }
        std::fs::write(version_dir.join(MANIFEST), manifest.to_string()).unwrap();
        std::fs::remove_file(content.join("alias")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("z-target", content.join("alias")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file("z-target", content.join("alias")).unwrap();
        assert!(verify_skill_version(&root.data(), &version).is_err());
        std::fs::remove_file(content.join("alias")).unwrap();
        std::fs::write(content.join("alias"), b"payload").unwrap();
        std::fs::remove_file(source.join("alias")).unwrap();
        std::fs::write(source.join("alias"), format!("symlink:{raw_target}")).unwrap();
        let file_version = source_versions(&source).unwrap().current;
        assert_ne!(version, file_version);
        let forged = root.data().join("skill-versions").join(&file_version);
        std::fs::rename(&version_dir, &forged).unwrap();
        let mut forged_manifest = manifest;
        forged_manifest["version"] = serde_json::json!(file_version);
        std::fs::write(forged.join(MANIFEST), forged_manifest.to_string()).unwrap();
        assert!(verify_skill_version(&root.data(), &file_version).is_err());
    }

    #[test]
    fn materialized_writer_policy_does_not_block_verified_schema2_reuse() {
        let root = Root::new();
        let source = root.source();
        std::fs::write(source.join("z-target"), b"payload").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("missing/../z-target", source.join("alias")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file("missing/../z-target", source.join("alias")).unwrap();
        let entries = source_entries(&source).unwrap();
        assert!(super::super::snapshot_materialized::plan(&entries).is_err());
        let version = typed_hash(&entries);
        let version_dir = root.data().join("skill-versions").join(&version);
        let content = version_dir.join("content");
        super::super::snapshot::copy_dir_recursive(&source, &content).unwrap();
        write_manifest(&version_dir, &version).unwrap();
        assert_eq!(
            snapshot_git_skill(&source, &root.data()).unwrap(),
            (version, content)
        );
    }

    #[test]
    fn typed_fingerprint_has_a_fixed_golden_and_preserves_legacy_hashing() {
        let root = Root::new();
        let source = root.source();
        std::fs::create_dir(source.join("assets")).unwrap();
        std::fs::create_dir(source.join("empty")).unwrap();
        std::fs::create_dir(source.join("node_modules")).unwrap();
        std::fs::write(source.join("assets/a.txt"), b"payload").unwrap();
        std::fs::write(source.join("node_modules/ignored"), b"excluded").unwrap();
        let versions = source_versions(&source).unwrap();
        assert_eq!(versions.legacy, hash_skill_dir(&source).unwrap());
        assert_eq!(
            versions.current,
            "cbdb7e5f37f28157c43ac53cd7c18783002803ab249e1a515202cb4f0b62c413"
        );
        assert_ne!(versions.current, versions.legacy);
        std::fs::remove_dir(source.join("empty")).unwrap();
        let without_empty = source_versions(&source).unwrap();
        assert_eq!(without_empty.legacy, versions.legacy);
        assert_ne!(without_empty.current, versions.current);
    }

    #[test]
    fn typed_and_legacy_snapshots_require_their_own_manifest_contract() {
        let root = Root::new();
        let old = hash_skill_dir(&root.source()).unwrap();
        let old_content = snapshot_skill(&root.source(), &root.data(), &old).unwrap();
        assert_eq!(
            verify_skill_version(&root.data(), &old).unwrap(),
            old_content
        );
        write_manifest(old_content.parent().unwrap(), &old).unwrap();
        assert!(verify_skill_version(&root.data(), &old).is_err());
        std::fs::remove_file(old_content.parent().unwrap().join(MANIFEST)).unwrap();
        assert_eq!(
            verify_skill_version(&root.data(), &old).unwrap(),
            old_content
        );
        let (current, content) = snapshot_git_skill(&root.source(), &root.data()).unwrap();
        assert_ne!(current, old);
        assert_eq!(
            snapshot_git_skill(&root.source(), &root.data()).unwrap(),
            (current.clone(), content.clone())
        );
        let manifest = content.parent().unwrap().join(MANIFEST);
        let original = std::fs::read(&manifest).unwrap();
        for invalid in [
            b"{".to_vec(),
            format!("{{\"schema\":3,\"version\":\"{current}\"}}").into_bytes(),
            format!("{{\"schema\":2,\"schema\":2,\"version\":\"{current}\"}}").into_bytes(),
            format!("{{\"schema\":2,\"version\":\"{current}\",\"aliases\":[]}}").into_bytes(),
            format!("{{\"schema\":2,\"version\":\"{old}\"}}").into_bytes(),
            format!(
                "{{\"schema\":2,\"version\":\"{current}\"}}{}",
                " ".repeat(4096)
            )
            .into_bytes(),
            vec![b' '; 4097],
        ] {
            std::fs::write(&manifest, invalid).unwrap();
            assert!(verify_skill_version(&root.data(), &current).is_err());
            assert!(snapshot_git_skill(&root.source(), &root.data()).is_err());
        }
        std::fs::remove_file(&manifest).unwrap();
        assert!(verify_skill_version(&root.data(), &current).is_err());
        std::fs::create_dir(&manifest).unwrap();
        assert!(verify_skill_version(&root.data(), &current).is_err());
        std::fs::remove_dir(&manifest).unwrap();
        let target = root.0.join("manifest-target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("marker"), b"external").unwrap();
        crate::platform::link_dir(&target, &manifest).unwrap();
        assert!(verify_skill_version(&root.data(), &current).is_err());
        assert_eq!(std::fs::read(target.join("marker")).unwrap(), b"external");
        crate::platform::remove_link(&manifest).unwrap();
        std::fs::write(&manifest, original).unwrap();
        assert_eq!(
            verify_skill_version(&root.data(), &current).unwrap(),
            content
        );
    }

    #[test]
    fn typed_fingerprint_distinguishes_magic_files_links_and_raw_targets() {
        #[cfg(unix)]
        use std::os::unix::fs::symlink as link_file;
        #[cfg(windows)]
        use std::os::windows::fs::symlink_file as link_file;
        let root = Root::new();
        let source = root.source();
        std::fs::write(source.join("target"), b"payload").unwrap();
        std::fs::write(source.join("alias"), b"symlink:target").unwrap();
        let file = source_versions(&source).unwrap();
        let (version, content) = snapshot_git_skill(&source, &root.data()).unwrap();
        std::fs::remove_file(source.join("alias")).unwrap();
        link_file("target", source.join("alias")).unwrap();
        let link = source_versions(&source).unwrap();
        let (link_version, link_content) = snapshot_git_skill(&source, &root.data()).unwrap();
        assert_eq!(link_version, link.current);
        assert_eq!(
            verify_skill_version(&root.data(), &link_version).unwrap(),
            link_content
        );
        assert_eq!(
            std::fs::read(link_content.join("alias")).unwrap(),
            b"payload"
        );
        assert_eq!(
            file.legacy, link.legacy,
            "The legacy ambiguity must remain visible in this regression"
        );
        assert_ne!(file.current, link.current);
        std::fs::remove_file(content.join("alias")).unwrap();
        link_file("target", content.join("alias")).unwrap();
        assert_eq!(hash_skill_dir(&content).unwrap(), file.legacy);
        assert!(verify_skill_version(&root.data(), &version).is_err());
        std::fs::remove_file(source.join("alias")).unwrap();
        link_file("./target", source.join("alias")).unwrap();
        let spelled = source_versions(&source).unwrap();
        assert_ne!(link.current, spelled.current);
        assert_eq!(
            target_key(Path::new("alias"), Path::new("target")).unwrap(),
            target_key(Path::new("alias"), Path::new("./target")).unwrap()
        );
        #[cfg(unix)]
        {
            assert_eq!(relative_key(Path::new("a\\b")).unwrap(), "a\\b");
            assert_eq!(
                target_key(Path::new("alias"), Path::new("a\\b")).unwrap(),
                "a\\b"
            );
        }
        #[cfg(windows)]
        {
            assert_eq!(relative_key(Path::new("a\\b")).unwrap(), "a/b");
            assert_eq!(
                target_key(Path::new("alias"), Path::new("a\\b")).unwrap(),
                "a/b"
            );
            assert!(target_key(Path::new("alias"), Path::new("C:target")).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn typed_path_keys_reject_lossy_unicode() {
        use std::os::unix::ffi::OsStringExt;
        let path = Path::new("nested").join(std::ffi::OsString::from_vec(vec![0xff]));
        assert!(relative_key(&path).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn typed_sources_refuse_absolute_namespaces_before_path_comparison_normalization() {
        let root = Root::new();
        let source = root.source();
        let alias = source.join("alias");
        for target in [
            r"\\?\Volume{00000000-0000-0000-0000-000000000000}\outside".to_string(),
            // Rust keeps an extended UNC prefix when its UTF-16 path exceeds
            // MAX_PATH; short UNC targets may be returned in ordinary UNC form.
            format!(
                r"\\?\UNC\YOUR_SERVER\YOUR_SHARE\{}\{}\outside",
                "a".repeat(130),
                "b".repeat(130)
            ),
        ] {
            std::os::windows::fs::symlink_file(&target, &alias).unwrap();
            let original = std::fs::read_link(&alias).unwrap();
            assert!(
                original.is_absolute(),
                "The fixture must retain an absolute namespace target"
            );
            let comparison = crate::platform::read_link(&alias).unwrap();
            assert!(
                !comparison.is_absolute(),
                "The legacy comparison helper exposes the original normalization defect"
            );
            assert!(
                hash_skill_dir(&source).is_ok(),
                "The legacy control must exercise its existing behavior"
            );
            assert!(source_versions(&source).is_err());
            assert!(snapshot_git_skill(&source, &root.data()).is_err());
            assert!(!root.data().exists());
            std::fs::remove_file(&alias).unwrap();
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn typed_paths_reject_non_utf8_files_without_changing_legacy_support() {
        use std::os::unix::ffi::OsStringExt;
        let root = Root::new();
        std::fs::write(
            root.source().join(std::ffi::OsString::from_vec(vec![0xff])),
            b"payload",
        )
        .unwrap();
        assert!(hash_skill_dir(&root.source()).is_ok());
        assert!(source_versions(&root.source()).is_err());
        assert!(snapshot_git_skill(&root.source(), &root.data()).is_err());
        assert!(!root.data().exists());
    }
}
