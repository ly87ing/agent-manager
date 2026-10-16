//! No-replace snapshot publication and cleanup guarded by observed directory identities.
use anyhow::{bail, Context as _, Result};
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DirectoryId(u64, [u8; 16]);

pub(super) fn directory_id(path: &Path) -> Result<DirectoryId> {
    Ok(open_directory(path)?.1)
}

pub(super) fn open_directory(path: &Path) -> Result<(File, DirectoryId)> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        };
        options
            .access_mode(FILE_READ_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(path)
        .with_context(|| format!("Cannot inspect snapshot directory {}", path.display()))?;
    let metadata = file.metadata()?;
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    };
    #[cfg(not(windows))]
    let reparse = metadata.file_type().is_symlink();
    if !metadata.is_dir() || reparse {
        bail!("Snapshot directory must be ordinary: {}", path.display());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let identity = DirectoryId(metadata.dev(), (metadata.ino() as u128).to_le_bytes());
        Ok((file, identity))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
        };
        let mut info = std::mem::MaybeUninit::<FILE_ID_INFO>::uninit();
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                info.as_mut_ptr().cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let info = unsafe { info.assume_init() };
        let identity = DirectoryId(info.VolumeSerialNumber, info.FileId.Identifier);
        let reference = windows_identity_reference(&file, identity)?;
        Ok((reference, identity))
    }
}

#[cfg(windows)]
fn windows_identity_reference(original: &File, expected: DirectoryId) -> Result<File> {
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows_sys::Win32::Storage::FileSystem::*;
    // Path-bound descendant handles prevent Windows directory rename even with
    // FILE_SHARE_DELETE. Keep the object referenced by its full file ID instead.
    let descriptor = FILE_ID_DESCRIPTOR {
        dwSize: std::mem::size_of::<FILE_ID_DESCRIPTOR>() as u32,
        Type: ExtendedFileIdType,
        Anonymous: FILE_ID_DESCRIPTOR_0 {
            ExtendedFileId: FILE_ID_128 {
                Identifier: expected.1,
            },
        },
    };
    let handle = unsafe {
        OpenFileById(
            original.as_raw_handle(),
            &descriptor,
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
        )
    };
    if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error())
            .context("Cannot retain snapshot directory by file ID");
    }
    let reference = unsafe { File::from_raw_handle(handle) };
    let metadata = reference.metadata()?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        bail!("Snapshot identity reference must be an ordinary directory");
    }
    let mut info = std::mem::MaybeUninit::<FILE_ID_INFO>::uninit();
    if unsafe {
        GetFileInformationByHandleEx(
            reference.as_raw_handle(),
            FileIdInfo,
            info.as_mut_ptr().cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let info = unsafe { info.assume_init() };
    if DirectoryId(info.VolumeSerialNumber, info.FileId.Identifier) != expected {
        bail!("Snapshot identity reference changed; refusing to continue");
    }
    Ok(reference)
}

pub(super) fn unchanged(path: &Path, expected: DirectoryId) -> Result<()> {
    if directory_id(path)? != expected {
        bail!(
            "Snapshot directory changed; preserving it: {}",
            path.display()
        );
    }
    Ok(())
}

fn rename_without_replace(source: &Path, target: &Path) -> Result<()> {
    #[cfg(unix)]
    let (source, target) = {
        use std::os::unix::ffi::OsStrExt;
        (
            std::ffi::CString::new(source.as_os_str().as_bytes())?,
            std::ffi::CString::new(target.as_os_str().as_bytes())?,
        )
    };
    #[cfg(target_os = "macos")]
    let success =
        unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) == 0 };
    #[cfg(target_os = "linux")]
    let success = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        ) == 0
    };
    #[cfg(windows)]
    let success = {
        use std::os::windows::ffi::OsStrExt;
        // Canonicalize only the shared parent for Win32's extended-length path.
        // Do not follow a replaced staging entry while preparing its rename.
        let parent = std::fs::canonicalize(source.parent().unwrap())?;
        let source = parent.join(source.file_name().unwrap());
        let target = parent.join(target.file_name().unwrap());
        let source: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<_> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                0,
            ) != 0
        }
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        bail!("Atomic snapshot publication is unavailable on this platform.");
    }
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    if !success {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

pub(super) struct Staging {
    path: PathBuf,
    parent: DirectoryId,
    identity: DirectoryId,
    content: DirectoryId,
    // Keep the original identities alive while checking paths that may be replaced.
    _parent_file: File,
    _directory_file: File,
    _content_file: File,
}

impl Staging {
    pub(super) fn create(parent: &Path, version: &str) -> Result<Self> {
        let (parent_file, parent_id) = open_directory(parent)?;
        let path = parent.join(format!(".staging-{version}-{}", crate::store::uuid_v4()));
        std::fs::create_dir(&path)?;
        // If identity capture fails, ownership is uncertain and cleanup must not guess.
        let (directory_file, identity) = open_directory(&path)?;
        let content_path = path.join("content");
        std::fs::create_dir(&content_path)?;
        let (content_file, content) = open_directory(&content_path)?;
        unchanged(parent, parent_id)?;
        unchanged(&path, identity)?;
        unchanged(&content_path, content)?;
        Ok(Self {
            path,
            parent: parent_id,
            identity,
            content,
            _parent_file: parent_file,
            _directory_file: directory_file,
            _content_file: content_file,
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    fn current(&self) -> Result<()> {
        unchanged(self.path.parent().unwrap(), self.parent)?;
        unchanged(&self.path, self.identity)?;
        unchanged(&self.path.join("content"), self.content)
    }

    pub(super) fn publish(&self, target: &Path) -> Result<()> {
        if target.parent() != self.path.parent() {
            bail!("Snapshot publication must keep the same parent directory.");
        }
        self.current()?;
        rename_without_replace(&self.path, target)?;
        unchanged(self.path.parent().unwrap(), self.parent)?;
        unchanged(target, self.identity)?;
        unchanged(&target.join("content"), self.content)
    }

    pub(super) fn cleanup(&self) -> Result<()> {
        self.current()?;
        #[cfg(windows)]
        super::fork::make_writable(&self.path)?;
        self.current()?;
        std::fs::remove_dir_all(&self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Root(PathBuf);
    impl Root {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("am-snapshot-storage-{}", crate::store::uuid_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn publication_never_replaces_an_existing_empty_directory() {
        let root = Root::new();
        let staging = Staging::create(&root.0, "fixture").unwrap();
        std::fs::write(staging.path().join("marker"), b"owned").unwrap();
        let target = root.0.join("existing");
        std::fs::create_dir(&target).unwrap();
        assert!(staging.publish(&target).is_err());
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        assert_eq!(
            std::fs::read(staging.path().join("marker")).unwrap(),
            b"owned"
        );
        staging.cleanup().unwrap();
        let staging = Staging::create(&root.0, "fixture").unwrap();
        std::fs::write(staging.path().join("marker"), b"accepted").unwrap();
        let target = root.0.join("new");
        staging.publish(&target).unwrap();
        assert!(!staging.path().exists());
        assert_eq!(std::fs::read(target.join("marker")).unwrap(), b"accepted");
    }

    #[test]
    fn replaced_staging_is_preserved_during_cleanup_and_publication() {
        let root = Root::new();
        let staging = Staging::create(&root.0, "fixture").unwrap();
        std::fs::write(staging.path().join("marker"), b"owned").unwrap();
        let moved = root.0.join("moved");
        std::fs::rename(staging.path(), &moved).unwrap();
        std::fs::create_dir(staging.path()).unwrap();
        std::fs::write(staging.path().join("marker"), b"external").unwrap();
        assert!(staging.cleanup().is_err());
        assert!(staging.publish(&root.0.join("version")).is_err());
        assert_eq!(
            std::fs::read(staging.path().join("marker")).unwrap(),
            b"external"
        );
        assert_eq!(std::fs::read(moved.join("marker")).unwrap(), b"owned");
        assert!(!root.0.join("version").exists());
    }

    #[test]
    fn replaced_content_is_preserved_during_cleanup_and_publication() {
        let root = Root::new();
        let staging = Staging::create(&root.0, "fixture").unwrap();
        let content = staging.path().join("content");
        std::fs::create_dir_all(&content).unwrap();
        std::fs::write(content.join("marker"), b"owned").unwrap();
        let moved = root.0.join("moved-content");
        std::fs::rename(&content, &moved).unwrap();
        std::fs::create_dir(&content).unwrap();
        std::fs::write(content.join("marker"), b"external").unwrap();
        assert!(staging.cleanup().is_err());
        assert!(staging.publish(&root.0.join("version")).is_err());
        assert_eq!(std::fs::read(content.join("marker")).unwrap(), b"external");
        assert_eq!(std::fs::read(moved.join("marker")).unwrap(), b"owned");
        assert!(!root.0.join("version").exists());
    }

    #[test]
    fn directory_links_never_grant_snapshot_directory_ownership() {
        let root = Root::new();
        let target = root.0.join("target");
        let link = root.0.join("link");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("marker"), b"untouched").unwrap();
        crate::platform::link_dir(&target, &link).unwrap();
        assert!(directory_id(&link).is_err());
        assert_eq!(std::fs::read(target.join("marker")).unwrap(), b"untouched");
        crate::platform::remove_link(link).unwrap();
    }

    #[test]
    fn replaced_parent_stops_staging_cleanup() {
        let root = Root::new();
        let parent = root.0.join("versions");
        std::fs::create_dir(&parent).unwrap();
        let staging = Staging::create(&parent, "fixture").unwrap();
        let relative = staging.path().file_name().unwrap().to_owned();
        std::fs::write(staging.path().join("marker"), b"owned").unwrap();
        let moved = root.0.join("moved");
        std::fs::rename(&parent, &moved).unwrap();
        std::fs::create_dir_all(parent.join(&relative)).unwrap();
        std::fs::write(parent.join(&relative).join("marker"), b"external").unwrap();
        assert!(staging.cleanup().is_err());
        assert!(staging.publish(&parent.join("version")).is_err());
        assert!(!parent.join("version").exists());
        assert_eq!(
            std::fs::read(parent.join(&relative).join("marker")).unwrap(),
            b"external"
        );
        assert_eq!(
            std::fs::read(moved.join(&relative).join("marker")).unwrap(),
            b"owned"
        );
    }

    #[cfg(windows)]
    #[test]
    fn long_paths_preserve_existing_targets_and_publish_new_versions() {
        let root = Root::new();
        let parent = root
            .0
            .join("a".repeat(100))
            .join("b".repeat(100))
            .join("c".repeat(100));
        std::fs::create_dir_all(&parent).unwrap();
        assert!(parent.as_os_str().len() > 260);
        let staging = Staging::create(&parent, "fixture").unwrap();
        std::fs::write(staging.path().join("content/marker"), b"owned").unwrap();
        let target = parent.join("existing");
        std::fs::create_dir(&target).unwrap();
        assert!(staging.publish(&target).is_err());
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        let target = parent.join("new");
        staging.publish(&target).unwrap();
        assert_eq!(
            std::fs::read(target.join("content/marker")).unwrap(),
            b"owned"
        );
    }
}
