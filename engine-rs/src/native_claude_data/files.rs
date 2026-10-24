//! Scoped, verified leaf removal without following external link targets.
use super::tree::{self, Entry, Kind};
use anyhow::{bail, Result};
use cap_std::fs::Dir;
#[cfg(windows)]
use cap_std::fs::OpenOptions;
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
        if expected.kind == Kind::File {
            validate_link_count(&file)?;
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

#[cfg(windows)]
pub(super) fn validate_single_link(parent: &Dir, name: &Path) -> Result<()> {
    validate_link_count(&parent.open(name)?)
}

#[cfg(unix)]
pub(super) fn validate_single_link(parent: &Dir, name: &Path) -> Result<()> {
    use cap_std::fs::MetadataExt;
    if parent.symlink_metadata(name)?.nlink() != 1 {
        bail!("A selected file has hard-link aliases; cleanup is unavailable.");
    }
    Ok(())
}

#[cfg(windows)]
fn validate_link_count(file: &cap_std::fs::File) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, info.as_mut_ptr()) } == 0 {
        bail!("A selected file link count could not be verified.");
    }
    if unsafe { info.assume_init() }.nNumberOfLinks != 1 {
        bail!("A selected file has unverified or hard-link aliases; cleanup is unavailable.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn a_new_hard_link_before_deletion_preserves_the_original_and_its_alias() {
        let (path, dir, entry) = fixture();
        std::fs::hard_link(path.join("settings.json"), path.join("retained-alias.json")).unwrap();
        assert!(remove(&dir, Path::new("settings.json"), &entry).is_err());
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
    fn a_changed_file_is_not_deleted() {
        let (path, dir, entry) = fixture();
        std::fs::write(path.join("settings.json"), b"NEW_CONFIGURATION").unwrap();
        assert!(remove(&dir, Path::new("settings.json"), &entry).is_err());
        assert_eq!(dir.read("settings.json").unwrap(), b"NEW_CONFIGURATION");
        drop(dir);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn deleting_a_link_preserves_external_bytes_and_mode() {
        use std::os::unix::fs::PermissionsExt;
        let (path, dir, _) = fixture();
        std::fs::set_permissions(
            path.join("settings.json"),
            std::fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        std::os::unix::fs::symlink(path.join("settings.json"), path.join("link")).unwrap();
        let link = tree::entry(&dir, Path::new("link"), "link".into()).unwrap();
        remove(&dir, Path::new("link"), &link).unwrap();
        assert_eq!(dir.read("settings.json").unwrap(), b"ORIGINAL_TELEMETRY");
        assert_eq!(
            std::fs::metadata(path.join("settings.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
        assert!(!dir.symlink_metadata("link").is_ok());
        drop(dir);
        std::fs::remove_dir_all(path).unwrap();
    }
}
