//! Scoped leaf removal and atomic replacement of telemetry-bearing configurations.
use super::tree::{self, Entry, Kind};
use anyhow::{bail, Result};
use cap_std::fs::{Dir, OpenOptions};
use std::io::Write;
use std::path::Path;

pub(super) fn remove(parent: &Dir, name: &Path, expected: &Entry) -> Result<()> {
    if expected.kind == Kind::Directory {
        if tree::identity(&parent.open_dir(name)?)? != expected.identity {
            bail!("A directory was replaced; cleanup stopped.");
        }
        if parent.open_dir(name)?.entries()?.next().is_some() {
            bail!("New directory contents appeared; cleanup stopped.");
        }
    } else if tree::entry(parent, name, expected.path.clone())?.hash != expected.hash {
        bail!("A file or link changed; cleanup stopped.");
    }
    #[cfg(unix)]
    {
        if expected.kind == Kind::Directory {
            parent.remove_dir(name)?;
        } else {
            parent.remove_file(name)?;
        }
    }
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::*;
        let mut options = OpenOptions::new();
        options
            .access_mode(DELETE | FILE_READ_ATTRIBUTES)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS);
        let file = parent.open_with(name, &options)?;
        let metadata = file.metadata()?;
        if tree::windows_identity(&file.try_clone()?.into_std())? != expected.identity {
            bail!("An item was replaced before removal; cleanup stopped.");
        }
        if expected.kind != Kind::Directory && tree::stamp(&metadata)? != expected.stamp {
            bail!("A file was replaced before removal; cleanup stopped.");
        }
        let info = FILE_DISPOSITION_INFO_EX {
            Flags: FILE_DISPOSITION_FLAG_DELETE
                | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS
                | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
        };
        if unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle() as _,
                FileDispositionInfoEx,
                (&info as *const FILE_DISPOSITION_INFO_EX).cast(),
                std::mem::size_of_val(&info) as u32,
            )
        } == 0
        {
            bail!("Windows refused removal of a verified item; cleanup stopped.");
        }
        drop(file);
    }
    match parent.symlink_metadata(name) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        _ => bail!("Removal could not be independently verified; cleanup stopped."),
    }
    Ok(())
}

pub(super) fn validate_replacement(parent: &Dir, name: &Path) -> Result<()> {
    #[cfg(windows)]
    validate_single_link(parent, name)?;
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let file = parent.open(name)?;
        let length = unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0) };
        if length != 0 {
            bail!("A telemetry configuration has extended metadata that cannot be preserved by this adapter.");
        }
    }
    #[cfg(windows)]
    if parent.symlink_metadata(name)?.permissions().readonly() {
        bail!("A read-only telemetry configuration cannot be replaced safely.");
    }
    let _ = (parent, name);
    Ok(())
}

#[cfg(windows)]
pub(super) fn validate_single_link(parent: &Dir, name: &Path) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let file = parent.open(name)?.into_std();
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, info.as_mut_ptr()) } == 0 {
        bail!("A settings file link count could not be verified.");
    }
    if unsafe { info.assume_init() }.nNumberOfLinks != 1 {
        bail!("A settings file has unverified or hard-link aliases; cleanup is unavailable.");
    }
    Ok(())
}

pub(super) fn replace(
    parent: &Dir,
    absolute_parent: &Path,
    name: &Path,
    expected: &Entry,
    bytes: &[u8],
) -> Result<()> {
    replace_with(
        parent,
        absolute_parent,
        name,
        expected,
        bytes,
        |_, _| Ok(()),
    )
}

fn replace_with(
    parent: &Dir,
    absolute_parent: &Path,
    name: &Path,
    expected: &Entry,
    bytes: &[u8],
    before_publish: impl FnOnce(&Dir, &str) -> Result<()>,
) -> Result<()> {
    #[cfg(windows)]
    let _parents = hold_windows_parents(absolute_parent, parent)?;
    if tree::entry(parent, name, expected.path.clone())?.hash != expected.hash {
        bail!("A telemetry configuration changed; request a new preview.");
    }
    validate_replacement(parent, name)?;
    let temporary = format!(".agent-manager-telemetry-{}", crate::store::uuid_v4());
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = parent.open_with(&temporary, &options)?;
    let candidate = tree::entry(parent, Path::new(&temporary), temporary.clone().into())?;
    let result = (|| -> Result<()> {
        #[cfg(unix)]
        let original = parent.open(name)?;
        #[cfg(target_os = "macos")]
        {
            use std::os::fd::AsRawFd;
            if unsafe {
                libc::fcopyfile(
                    original.as_raw_fd(),
                    file.as_raw_fd(),
                    std::ptr::null_mut(),
                    libc::COPYFILE_METADATA,
                )
            } != 0
            {
                bail!("Telemetry file metadata could not be preserved.");
            }
        }
        #[cfg(target_os = "linux")]
        file.set_permissions(original.metadata()?.permissions())?;
        file.write_all(bytes)?;
        file.sync_all()?;
        before_publish(parent, &temporary)?;
        if tree::entry(parent, name, expected.path.clone())?.hash != expected.hash {
            bail!("A telemetry configuration changed before replacement; cleanup stopped.");
        }
        validate_replacement(parent, name)?;
        let staged = tree::entry(parent, Path::new(&temporary), temporary.clone().into())?;
        if staged.identity != candidate.identity
            || staged.kind != Kind::File
            || staged.bytes != bytes.len() as u64
        {
            bail!("The telemetry candidate was replaced; cleanup stopped.");
        }
        use std::io::Read;
        let mut staged_bytes = Vec::new();
        parent
            .open(&temporary)?
            .take(bytes.len() as u64 + 1)
            .read_to_end(&mut staged_bytes)?;
        if staged_bytes != bytes {
            bail!("The telemetry candidate changed; cleanup stopped.");
        }
        drop(file);
        #[cfg(unix)]
        parent.rename(&temporary, parent, name)?;
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;
            // Keep every ancestor in place while ReplaceFileW preserves the destination DACL.
            let target: Vec<_> = absolute_parent
                .join(name)
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            let candidate: Vec<_> = absolute_parent
                .join(&temporary)
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            if unsafe {
                ReplaceFileW(
                    target.as_ptr(),
                    candidate.as_ptr(),
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                    std::ptr::null(),
                )
            } == 0
            {
                bail!("Windows could not atomically preserve the telemetry configuration.");
            }
        }
        let mut actual = Vec::new();
        parent
            .open(name)?
            .take(bytes.len() as u64 + 1)
            .read_to_end(&mut actual)?;
        if actual != bytes {
            bail!("Telemetry configuration readback failed; cleanup stopped.");
        }
        Ok(())
    })();
    if parent.symlink_metadata(&temporary).is_ok() {
        // The candidate contains retained telemetry fields only, never an account backup.
        let current = tree::entry(parent, Path::new(&temporary), temporary.clone().into())?;
        if current.identity != candidate.identity {
            bail!("A staging file was replaced; it was retained for inspection.");
        }
        parent.remove_file(&temporary)?;
    }
    let _ = absolute_parent;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn a_new_hard_link_before_publication_preserves_the_original_and_its_alias() {
        let (path, dir, entry) = fixture();
        let result = replace_with(
            &dir,
            &path,
            Path::new("settings.json"),
            &entry,
            b"RETAINED",
            |_, _| {
                std::fs::hard_link(path.join("settings.json"), path.join("retained-alias.json"))?;
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(dir.read("settings.json").unwrap(), b"ORIGINAL_TELEMETRY");
        assert_eq!(
            dir.read("retained-alias.json").unwrap(),
            b"ORIGINAL_TELEMETRY"
        );
        drop(dir);
        std::fs::remove_dir_all(path).unwrap();
    }
    fn fixture() -> (std::path::PathBuf, Dir, Entry) {
        let path =
            std::env::temp_dir().join(format!("am-claude-files-{}", crate::store::uuid_v4()));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("settings.json"), b"ORIGINAL_TELEMETRY").unwrap();
        let dir = Dir::open_ambient_dir(&path, cap_std::ambient_authority()).unwrap();
        let entry = tree::entry(&dir, Path::new("settings.json"), "settings.json".into()).unwrap();
        (path, dir, entry)
    }
    #[cfg(windows)]
    #[test]
    fn read_only_files_and_external_junctions_remove_only_the_selected_entry() {
        let (path, dir, _) = fixture();
        let mut permissions = std::fs::metadata(path.join("settings.json"))
            .unwrap()
            .permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(path.join("settings.json"), permissions).unwrap();
        let entry = tree::entry(&dir, Path::new("settings.json"), "settings.json".into()).unwrap();
        remove(&dir, Path::new("settings.json"), &entry).unwrap();
        let outside =
            path.with_file_name(format!("am-claude-external-{}", crate::store::uuid_v4()));
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("source"), b"EXTERNAL_SOURCE").unwrap();
        junction::create(&outside, path.join("link")).unwrap();
        let entry = tree::entry(&dir, Path::new("link"), "link".into()).unwrap();
        assert_eq!(entry.kind, Kind::Link);
        remove(&dir, Path::new("link"), &entry).unwrap();
        assert_eq!(
            std::fs::read(outside.join("source")).unwrap(),
            b"EXTERNAL_SOURCE"
        );
        drop(dir);
        std::fs::remove_dir_all(path).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }
    #[test]
    fn changed_file_and_replaced_candidate_do_not_overwrite_the_original() {
        let (path, dir, entry) = fixture();
        std::fs::write(path.join("settings.json"), b"NEW_TELEMETRY").unwrap();
        assert!(remove(&dir, Path::new("settings.json"), &entry).is_err());
        assert!(replace(&dir, &path, Path::new("settings.json"), &entry, b"RETAINED").is_err());
        let entry = tree::entry(&dir, Path::new("settings.json"), "settings.json".into()).unwrap();
        let result = replace_with(
            &dir,
            &path,
            Path::new("settings.json"),
            &entry,
            b"RETAINED",
            |dir, name| {
                dir.rename(name, dir, "held-candidate")?;
                dir.write(name, b"EXTERNAL_REPLACEMENT")?;
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(dir.read("settings.json").unwrap(), b"NEW_TELEMETRY");
        assert!(dir.entries().unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".agent-manager-telemetry-")));
        drop(dir);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn deleting_a_link_preserves_external_bytes_and_replacement_preserves_mode() {
        use std::os::unix::fs::PermissionsExt;
        let (path, dir, _) = fixture();
        std::fs::set_permissions(
            path.join("settings.json"),
            std::fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        let entry = tree::entry(&dir, Path::new("settings.json"), "settings.json".into()).unwrap();
        replace(
            &dir,
            &path,
            Path::new("settings.json"),
            &entry,
            b"RETAINED_TELEMETRY",
        )
        .unwrap();
        assert_eq!(dir.read("settings.json").unwrap(), b"RETAINED_TELEMETRY");
        assert_eq!(
            std::fs::metadata(path.join("settings.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
        std::os::unix::fs::symlink(path.join("settings.json"), path.join("link")).unwrap();
        let link = tree::entry(&dir, Path::new("link"), "link".into()).unwrap();
        remove(&dir, Path::new("link"), &link).unwrap();
        assert_eq!(dir.read("settings.json").unwrap(), b"RETAINED_TELEMETRY");
        assert!(!dir.symlink_metadata("link").is_ok());
        drop(dir);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(windows)]
fn hold_windows_parents(path: &Path, expected: &Dir) -> Result<Vec<std::fs::File>> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let mut current = std::path::PathBuf::new();
    let mut handles = Vec::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(component, std::path::Component::Prefix(_)) {
            continue;
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&current)?;
        use std::os::windows::fs::MetadataExt;
        let metadata = file.metadata()?;
        if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
            bail!("A replacement parent is redirected or is not a directory.");
        }
        handles.push(file);
    }
    if tree::windows_identity(
        handles
            .last()
            .ok_or_else(|| anyhow::anyhow!("No replacement parent."))?,
    )? != tree::identity(expected)?
    {
        bail!("A replacement parent changed; cleanup stopped.");
    }
    Ok(handles)
}
