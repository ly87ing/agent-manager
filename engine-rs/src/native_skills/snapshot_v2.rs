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
    let mut legacy = Vec::new();
    let mut entries = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    let mut buffer = [0u8; 64 * 1024];
    while let Some(directory) = stack.pop() {
        for child in std::fs::read_dir(&directory)? {
            let path = child?.path();
            let metadata = std::fs::symlink_metadata(&path)?;
            if metadata.is_dir()
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
            let legacy_key = relative.to_string_lossy().replace('\\', "/");
            let (kind, digest, target) = if metadata.file_type().is_symlink() {
                let raw = std::fs::read_link(&path)?;
                validate_symlink_internal(root, &path, &raw)?;
                let raw_text = raw
                    .to_str()
                    .context("Skill link targets must be UTF-8 for typed snapshots")?;
                let mut old = Sha256::new();
                old.update(b"symlink:");
                old.update(raw_text.as_bytes());
                legacy.push((legacy_key, format!("{:x}", old.finalize())));
                (
                    b'l',
                    format!("{:x}", Sha256::digest(raw_text.as_bytes())),
                    target_key(relative, &raw)?,
                )
            } else if metadata.is_dir() {
                stack.push(path.clone());
                (b'd', String::new(), String::new())
            } else if metadata.is_file() {
                let mut file = std::fs::File::open(&path)?;
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
                let digest = format!("{:x}", hash.finalize());
                legacy.push((legacy_key, digest.clone()));
                (b'f', digest, String::new())
            } else {
                bail!("Unsupported entry in typed skill snapshot");
            };
            if entries.insert(key, (kind, digest, target)).is_some() {
                bail!("Duplicate path in typed skill snapshot");
            }
        }
    }
    let mut hash = Sha256::new();
    hash.update(b"\0agent-manager-skill-v2\0");
    for (path, (kind, digest, target)) in entries {
        hash.update([kind, 0]);
        for value in [path, digest, target] {
            hash.update(value.as_bytes());
            hash.update([0]);
        }
    }
    Ok(SourceVersions {
        legacy: super::snapshot::hash_entries(legacy),
        current: format!("{:x}", hash.finalize()),
    })
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    version: String,
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

/// Only an absent manifest selects the legacy format; invalid metadata never does.
pub(super) fn is_typed(version_dir: &Path, version: &str) -> Result<bool> {
    let path = version_dir.join(MANIFEST);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
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
    if !metadata.is_file() || metadata.len() > 4096 {
        bail!("Snapshot manifest must be an ordinary file of at most 4096 bytes");
    }
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        bail!("Snapshot manifest exceeds 4096 bytes");
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    if manifest.schema != 2 || manifest.version != version {
        bail!("Snapshot manifest schema or version does not match");
    }
    Ok(true)
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
