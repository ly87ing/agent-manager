//! macOS file publication: preserve metadata and verify the atomically displaced file.
use super::{connection_clear::Plan, connection_inspect, tree};
use anyhow::{bail, Result};
use cap_std::fs::{Dir, OpenOptions, OpenOptionsExt};
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    io::{Read, Write},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::Path,
};

unsafe extern "C" {
    fn acl_get_fd_np(fd: i32, kind: u32) -> *mut libc::c_void;
    fn acl_size(acl: *mut libc::c_void) -> isize;
    fn acl_copy_ext(buffer: *mut libc::c_void, acl: *mut libc::c_void, size: isize) -> isize;
    fn acl_free(value: *mut libc::c_void) -> i32;
    fn acl_init(count: i32) -> *mut libc::c_void;
    fn acl_set_fd_np(fd: i32, acl: *mut libc::c_void, kind: u32) -> i32;
    fn fclonefileat(source: i32, directory: i32, name: *const libc::c_char, flags: u32) -> i32;
}

fn acl_bytes(acl: *mut libc::c_void) -> Result<Vec<u8>> {
    if acl.is_null() {
        bail!(
            "File permissions are unavailable: {}",
            std::io::Error::last_os_error()
        );
    }
    let size = unsafe { acl_size(acl) };
    let mut bytes = vec![0; size.clamp(0, 65536) as usize];
    let copied = if (0..=65536).contains(&size) {
        unsafe { acl_copy_ext(bytes.as_mut_ptr().cast(), acl, size) }
    } else {
        -1
    };
    unsafe {
        acl_free(acl);
    }
    if copied < 0 {
        bail!("File permissions are unavailable.");
    }
    bytes.truncate(copied as usize);
    Ok(bytes)
}

fn read_acl(fd: i32) -> Result<Vec<u8>> {
    let acl = unsafe { acl_get_fd_np(fd, 0x100) };
    // An open Darwin fd without an extended ACL reports ENOENT.
    if acl.is_null() && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
        return acl_bytes(unsafe { acl_init(0) });
    }
    acl_bytes(acl)
}

fn metadata(file: &std::fs::File) -> Result<Vec<u8>> {
    use std::os::macos::fs::MetadataExt as _;
    let stat = file.metadata()?;
    let mut hash = Sha256::new();
    hash.update(stat.mode().to_le_bytes());
    hash.update(stat.uid().to_le_bytes());
    hash.update(stat.gid().to_le_bytes());
    hash.update(stat.st_flags().to_le_bytes());
    let size = unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0, 0) };
    if !(0..=65536).contains(&size) {
        bail!("File attributes are unavailable or exceed the inspection bound.");
    }
    let mut names = vec![0u8; size as usize];
    if unsafe { libc::flistxattr(file.as_raw_fd(), names.as_mut_ptr().cast(), names.len(), 0) }
        != size
    {
        bail!("File attributes changed.");
    }
    let mut names: Vec<_> = names.split(|b| *b == 0).filter(|n| !n.is_empty()).collect();
    names.sort();
    for name in names {
        let key = CString::new(name)?;
        let size = unsafe {
            libc::fgetxattr(
                file.as_raw_fd(),
                key.as_ptr(),
                std::ptr::null_mut(),
                0,
                0,
                0,
            )
        };
        if !(0..=262144).contains(&size) {
            bail!("File attributes are unavailable or exceed the inspection bound.");
        }
        let mut value = vec![0u8; size as usize];
        if unsafe {
            libc::fgetxattr(
                file.as_raw_fd(),
                key.as_ptr(),
                value.as_mut_ptr().cast(),
                value.len(),
                0,
                0,
            )
        } != size
        {
            bail!("File attributes changed.");
        }
        hash.update(name);
        hash.update(size.to_le_bytes());
        hash.update(value);
    }
    hash.update(read_acl(file.as_raw_fd())?);
    Ok(hash.finalize().to_vec())
}

fn open(dir: &Dir, name: &str, write: bool) -> Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(write)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    Ok(dir.open_with(name, &options)?.into_std())
}

fn supported_files(source: &std::fs::File, _parent: &std::fs::File) -> Result<()> {
    let stat = source.metadata()?;
    if !stat.is_file()
        || stat.uid() != unsafe { libc::geteuid() }
        || stat.nlink() != 1
        || stat.mode() & 0o7000 != 0
    {
        bail!("Unsupported file ownership or permissions.");
    }
    metadata(source)?;
    Ok(())
}

pub(super) fn supported(path: &Path) -> Result<()> {
    crate::native_mcp::targets::validate_target_path(path)?;
    let dir = Dir::open_ambient_dir(path.parent().unwrap(), cap_std::ambient_authority())?;
    supported_files(
        &open(&dir, path.file_name().unwrap().to_str().unwrap(), false)?,
        &dir.into_std_file(),
    )
}

fn identity(file: &std::fs::File) -> Result<Vec<u8>> {
    let stat = file.metadata()?;
    Ok([stat.dev().to_le_bytes(), stat.ino().to_le_bytes()].concat())
}

fn matches(dir: &Dir, name: &str, id: &[u8], bytes: &[u8], attributes: &[u8]) -> bool {
    (|| -> Result<bool> {
        let file = open(dir, name, false)?;
        Ok(identity(&file)? == id && dir.read(name)? == bytes && metadata(&file)? == attributes)
    })()
    .unwrap_or(false)
}

fn swap(dir: &Dir, a: &str, target: &Dir, b: &str) -> Result<()> {
    let file = dir.try_clone()?.into_std_file();
    let target_file = target.try_clone()?.into_std_file();
    let a = CString::new(a)?;
    let b = CString::new(b)?;
    if unsafe {
        libc::renameatx_np(
            file.as_raw_fd(),
            a.as_ptr(),
            target_file.as_raw_fd(),
            b.as_ptr(),
            2,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn private_stage(parent: &Dir) -> Result<(String, Dir, Vec<u8>)> {
    let name = format!(".agent-manager-connection-{}", crate::store::uuid_v4());
    parent.create_dir(&name)?;
    let stage = parent.open_dir(&name)?;
    let identity = tree::identity(&stage)?;
    let file = stage.try_clone()?.into_std_file();
    let acl = unsafe { acl_init(0) };
    if acl.is_null() {
        bail!("Temporary directory permissions are unavailable.");
    }
    let set = unsafe { acl_set_fd_np(file.as_raw_fd(), acl, 0x100) };
    unsafe {
        acl_free(acl);
    }
    if set != 0
        || unsafe { libc::fchmod(file.as_raw_fd(), 0o700) } != 0
        || read_acl(file.as_raw_fd())? != acl_bytes(unsafe { acl_init(0) })?
        || file.metadata()?.mode() & 0o777 != 0o700
    {
        let _ = parent.remove_dir(&name);
        bail!("Temporary directory could not be restricted before preparing configuration.");
    }
    Ok((name, stage, identity))
}

pub(super) fn publish(plan: &Plan, guard: impl Fn() -> Result<()>) -> Result<()> {
    publish_with(plan, guard, || Ok(()), |_, _| Ok(()))
}

fn publish_with(
    plan: &Plan,
    guard: impl Fn() -> Result<()>,
    before_swap: impl FnOnce() -> Result<()>,
    before_unlink: impl FnOnce(&Dir, &str) -> Result<()>,
) -> Result<()> {
    supported(&plan.path)?;
    guard()?;
    let parent = plan.path.parent().unwrap();
    let dir = Dir::open_ambient_dir(parent, cap_std::ambient_authority())?;
    if tree::identity(&dir)? != plan.parent_identity {
        bail!("The settings directory changed.");
    }
    let original = tree::entry(
        &dir,
        Path::new(&plan.filename),
        plan.filename.clone().into(),
    )?;
    let source = open(&dir, &plan.filename, false)?;
    let dir_handle = dir.try_clone()?.into_std_file();
    supported_files(&source, &dir_handle)?;
    let mut original_bytes = Vec::new();
    source
        .try_clone()?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut original_bytes)?;
    if identity(&source)? != original.identity || original_bytes != plan.bytes {
        bail!("The source changed before preparation.");
    }
    let attributes = metadata(&source)?;
    let (stage_name, stage, stage_identity) = private_stage(&dir)?;
    let stage_handle = stage.try_clone()?.into_std_file();
    let recovery = parent.join(&stage_name).join("configuration");
    let temp = "configuration".to_string();
    let name = CString::new(temp.as_str())?;
    if unsafe {
        fclonefileat(
            source.as_raw_fd(),
            stage_handle.as_raw_fd(),
            name.as_ptr(),
            4,
        )
    } != 0
    {
        if stage.symlink_metadata(&temp).is_ok() {
            bail!(
                "Preparation is uncertain; inspect temporary file {}.",
                recovery.display()
            );
        }
        let error = std::io::Error::last_os_error();
        if dir.remove_dir(&stage_name).is_err() {
            bail!(
                "Preparation failed; an empty temporary directory remains at {}.",
                parent.join(&stage_name).display()
            );
        }
        return Err(error.into());
    }
    let staged_id = match open(&stage, &temp, false).and_then(|file| identity(&file)) {
        Ok(id) => id,
        Err(_) => bail!(
            "A temporary configuration remains at {}.",
            recovery.display()
        ),
    };
    let mut displaced = false;
    let result = (|| {
        if staged_id == original.identity
            || !matches(&stage, &temp, &staged_id, &plan.bytes, &attributes)
        {
            bail!("The prepared copy changed.");
        }
        let mut file = open(&stage, &temp, true)?;
        if identity(&file)? != staged_id {
            bail!("The prepared copy was replaced.");
        }
        file.write_all(&plan.output)?;
        file.set_len(plan.output.len() as u64)?;
        file.sync_all()?;
        if !matches(&stage, &temp, &staged_id, &plan.output, &attributes) {
            bail!("Prepared content or metadata did not verify.");
        }
        guard()?;
        if tree::entry(
            &dir,
            Path::new(&plan.filename),
            plan.filename.clone().into(),
        )?
        .hash
            != original.hash
            || !matches(
                &dir,
                &plan.filename,
                &original.identity,
                &plan.bytes,
                &attributes,
            )
        {
            bail!("The source changed before publication.");
        }
        before_swap()?;
        swap(&stage, &temp, &dir, &plan.filename)?;
        displaced = true;
        if !matches(&stage, &temp, &original.identity, &plan.bytes, &attributes) {
            let external = open(&stage, &temp, false)?;
            let external_id = identity(&external)?;
            let external_bytes = stage.read(&temp)?;
            let external_meta = metadata(&external)?;
            if matches(&dir, &plan.filename, &staged_id, &plan.output, &attributes) {
                swap(&stage, &temp, &dir, &plan.filename)?;
                if matches(
                    &dir,
                    &plan.filename,
                    &external_id,
                    &external_bytes,
                    &external_meta,
                ) && matches(&stage, &temp, &staged_id, &plan.output, &attributes)
                {
                    displaced = false;
                }
            }
            bail!("Concurrent settings changes were detected; request a new preview.");
        }
        if tree::identity(&Dir::open_ambient_dir(
            parent,
            cap_std::ambient_authority(),
        )?)? != plan.parent_identity
            || connection_inspect::read(&plan.path)?.as_deref() != Some(plan.output.as_slice())
        {
            bail!("Published settings could not be independently verified.");
        }
        before_unlink(&stage, &temp)?;
        if !matches(&stage, &temp, &original.identity, &plan.bytes, &attributes)
            || !matches(&dir, &plan.filename, &staged_id, &plan.output, &attributes)
        {
            bail!(
                "Published or displaced configuration changed; review the retained temporary file."
            );
        }
        dir_handle.sync_all()?;
        stage.remove_file(&temp)?;
        displaced = false;
        dir_handle.sync_all()?;
        Ok(())
    })();
    let mut retained = displaced;
    if !displaced && stage.symlink_metadata(&temp).is_ok() {
        let owned = matches(&stage, &temp, &staged_id, &plan.bytes, &attributes)
            || matches(&stage, &temp, &staged_id, &plan.output, &attributes);
        retained = !owned || stage.remove_file(&temp).is_err();
    }
    if !retained
        && (tree::identity(&dir.open_dir(&stage_name)?)? != stage_identity
            || dir.remove_dir(&stage_name).is_err())
    {
        retained = true;
    }
    if retained {
        bail!("Publication is uncertain. A temporary configuration remains at {}; inspect it before continuing.",recovery.display());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (std::path::PathBuf, Plan) {
        let root =
            std::env::temp_dir().join(format!("connection-publish-{}", crate::store::uuid_v4()));
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::create_dir(root.join(".codex")).unwrap();
        std::fs::write(root.join(".codex/auth.json"), b"PRIVATE_CODEX_SENTINEL").unwrap();
        let path = root.join(".claude/settings.json");
        let bytes=br#"{ "env": {"ANTHROPIC_API_KEY":"PRIVATE_SELECTED", "KEEP":"PRIVATE_RETAINED"}, "mcpServers":{}, "hooks": { "custom": [1,2] } }"#.to_vec();
        std::fs::write(&path, &bytes).unwrap();
        let parent =
            Dir::open_ambient_dir(path.parent().unwrap(), cap_std::ambient_authority()).unwrap();
        let output = super::super::connection_json::remove(&bytes, "ANTHROPIC_API_KEY").unwrap();
        (
            root,
            Plan {
                path,
                filename: "settings.json".into(),
                key: "ANTHROPIC_API_KEY".into(),
                item: "settings:ANTHROPIC_API_KEY".into(),
                bytes,
                output,
                base: String::new(),
                kind: "settings",
                line: None,
                root: None,
                parent_identity: tree::identity(&parent).unwrap(),
                blockers: vec![],
            },
        )
    }
    #[test]
    fn native_swap_preserves_unselected_bytes_metadata_and_codex() {
        use std::os::unix::fs::PermissionsExt;
        let (root, plan) = fixture();
        std::fs::set_permissions(&plan.path, std::fs::Permissions::from_mode(0o640)).unwrap();
        let name = CString::new("com.agent-manager.fixture").unwrap();
        let path = CString::new(plan.path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(
            unsafe { libc::setxattr(path.as_ptr(), name.as_ptr(), b"X".as_ptr().cast(), 1, 0, 0) },
            0
        );
        publish(&plan, || Ok(())).unwrap();
        assert_eq!(std::fs::read(&plan.path).unwrap(), plan.output);
        assert_eq!(std::fs::metadata(&plan.path).unwrap().mode() & 0o777, 0o640);
        let mut value = [0u8; 1];
        assert_eq!(
            unsafe {
                libc::getxattr(
                    path.as_ptr(),
                    name.as_ptr(),
                    value.as_mut_ptr().cast(),
                    1,
                    0,
                    0,
                )
            },
            1
        );
        assert_eq!(value, [b'X']);
        assert_eq!(
            std::fs::read(root.join(".codex/auth.json")).unwrap(),
            b"PRIVATE_CODEX_SENTINEL"
        );
        assert_eq!(std::fs::read_dir(root.join(".claude")).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn compare_to_swap_race_preserves_the_displaced_external_update() {
        let (root, plan) = fixture();
        let external = br#"{"env":{"ANTHROPIC_API_KEY":"PRIVATE_NEW"},"hooks":{"new":true}}"#;
        assert!(publish_with(
            &plan,
            || Ok(()),
            || {
                std::fs::write(&plan.path, external)?;
                Ok(())
            },
            |_, _| Ok(())
        )
        .is_err());
        assert_eq!(std::fs::read(&plan.path).unwrap(), external);
        assert_eq!(std::fs::read_dir(root.join(".claude")).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn late_writer_on_displaced_inode_retains_recovery_and_never_reports_complete() {
        let (root, plan) = fixture();
        let late = b"PRIVATE_LATE_UPDATE";
        assert!(publish_with(
            &plan,
            || Ok(()),
            || Ok(()),
            |dir, temp| {
                dir.write(temp, late)?;
                Ok(())
            }
        )
        .is_err());
        assert!(std::fs::read_dir(root.join(".claude"))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .any(|entry| std::fs::read(entry.path().join("configuration"))
                .is_ok_and(|bytes| bytes == late)));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn changed_parent_and_preparation_failure_preserve_original() {
        let (root, mut plan) = fixture();
        plan.parent_identity.push(1);
        assert!(publish(&plan, || Ok(())).is_err());
        assert_eq!(std::fs::read(&plan.path).unwrap(), plan.bytes);
        plan.parent_identity.pop();
        assert!(publish_with(&plan, || Ok(()), || bail!("injected stop"), |_, _| Ok(())).is_err());
        assert_eq!(std::fs::read(&plan.path).unwrap(), plan.bytes);
        assert_eq!(std::fs::read_dir(root.join(".claude")).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn inherited_acl_is_removed_from_stage_before_cloning_and_source_acl_is_retained() {
        let (root, plan) = fixture();
        for (path, acl) in [
            (
                plan.path.parent().unwrap(),
                "everyone allow read,execute,file_inherit,directory_inherit",
            ),
            (plan.path.as_path(), "everyone allow read"),
        ] {
            assert!(std::process::Command::new("/bin/chmod")
                .args(["+a", acl])
                .arg(path)
                .status()
                .unwrap()
                .success());
        }
        let dir = Dir::open_ambient_dir(plan.path.parent().unwrap(), cap_std::ambient_authority())
            .unwrap();
        let original_metadata = metadata(&open(&dir, &plan.filename, false).unwrap()).unwrap();
        publish_with(
            &plan,
            || Ok(()),
            || {
                let stages: Vec<_> = std::fs::read_dir(plan.path.parent().unwrap())?
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".agent-manager-connection-")
                    })
                    .collect();
                assert_eq!(stages.len(), 1);
                let stage = std::fs::File::open(stages[0].path())?;
                assert_eq!(stage.metadata()?.mode() & 0o777, 0o700);
                assert_eq!(
                    read_acl(stage.as_raw_fd())?,
                    acl_bytes(unsafe { acl_init(0) })?
                );
                Ok(())
            },
            |_, _| Ok(()),
        )
        .unwrap();
        assert_eq!(
            metadata(&open(&dir, &plan.filename, false).unwrap()).unwrap(),
            original_metadata
        );
        assert_eq!(std::fs::read(&plan.path).unwrap(), plan.output);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn replacement_of_displaced_name_is_retained_for_review() {
        let (root, plan) = fixture();
        assert!(publish_with(
            &plan,
            || Ok(()),
            || Ok(()),
            |stage, temp| {
                stage.rename(temp, stage, "external-original")?;
                stage.write(temp, b"PRIVATE_EXTERNAL_REPLACEMENT")?;
                Ok(())
            }
        )
        .is_err());
        let stage = std::fs::read_dir(root.join(".claude"))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .find(|entry| entry.path().is_dir())
            .unwrap()
            .path();
        assert_eq!(
            std::fs::read(stage.join("configuration")).unwrap(),
            b"PRIVATE_EXTERNAL_REPLACEMENT"
        );
        assert_eq!(
            std::fs::read(stage.join("external-original")).unwrap(),
            plan.bytes
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
