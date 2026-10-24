//! Bounded, no-follow evidence rooted in an already opened directory.
use anyhow::{bail, Context as _, Result};
use cap_std::fs::{Dir, Metadata};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Kind {
    Directory,
    File,
    Link,
}

#[derive(Clone, Debug)]
pub(super) struct Entry {
    pub path: PathBuf,
    pub kind: Kind,
    pub bytes: u64,
    #[cfg(windows)]
    pub stamp: Vec<u8>,
    pub hash: Vec<u8>,
    pub identity: Vec<u8>,
}

pub(super) fn stamp(metadata: &Metadata) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        bytes.extend(metadata.dev().to_le_bytes());
        bytes.extend(metadata.ino().to_le_bytes());
        bytes.extend(metadata.mode().to_le_bytes());
        bytes.extend(metadata.nlink().to_le_bytes());
        bytes.extend(metadata.uid().to_le_bytes());
        bytes.extend(metadata.gid().to_le_bytes());
        bytes.extend(metadata.ctime().to_le_bytes());
        bytes.extend(metadata.ctime_nsec().to_le_bytes());
    }
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        bytes.extend(metadata.creation_time().to_le_bytes());
        bytes.extend(metadata.file_attributes().to_le_bytes());
    }
    bytes.extend(metadata.len().to_le_bytes());
    bytes.extend(format!("{:?}", metadata.modified()?).as_bytes());
    Ok(bytes)
}

fn same_device(root: &Metadata, child: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        root.dev() == child.dev()
    }
    #[cfg(windows)]
    {
        let _ = (root, child);
        true
    }
}

pub(super) fn identity(dir: &Dir) -> Result<Vec<u8>> {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        let metadata = dir.dir_metadata()?;
        let identity = [metadata.dev().to_le_bytes(), metadata.ino().to_le_bytes()].concat();
        #[cfg(target_os = "linux")]
        let identity = [
            identity,
            mount_id(&dir.try_clone()?.into_std_file())?
                .to_le_bytes()
                .to_vec(),
        ]
        .concat();
        Ok(identity)
    }
    #[cfg(windows)]
    {
        windows_identity(&dir.try_clone()?.into_std_file())
    }
}

pub(super) fn same_mount(parent: &Dir, child: &Dir) -> Result<()> {
    #[cfg(target_os = "linux")]
    if mount_id(&parent.try_clone()?.into_std_file())?
        != mount_id(&child.try_clone()?.into_std_file())?
    {
        bail!("A mounted directory is outside the cleanup scope.");
    }
    let _ = (parent, child);
    Ok(())
}

#[cfg(target_os = "linux")]
fn mount_id(file: &std::fs::File) -> Result<u64> {
    use std::os::fd::AsRawFd;
    let mut bytes = Vec::new();
    std::fs::File::open(format!("/proc/self/fdinfo/{}", file.as_raw_fd()))?
        .take(65537)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        bail!("Mount identity is unavailable.");
    }
    let text = std::str::from_utf8(&bytes)?;
    let mut ids = text
        .lines()
        .filter_map(|line| line.strip_prefix("mnt_id:"))
        .map(str::trim);
    let id = ids
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|id| *id > 0);
    if id.is_none() || ids.next().is_some() {
        bail!("Mount identity is unavailable.");
    }
    Ok(id.unwrap())
}

#[cfg(windows)]
pub(super) fn windows_identity(file: &std::fs::File) -> Result<Vec<u8>> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, info.as_mut_ptr()) } == 0 {
        bail!("File identity could not be verified.");
    }
    let info = unsafe { info.assume_init() };
    Ok([
        info.dwVolumeSerialNumber.to_le_bytes(),
        info.nFileIndexHigh.to_le_bytes(),
        info.nFileIndexLow.to_le_bytes(),
    ]
    .concat())
}

pub(super) fn entry(parent: &Dir, name: &Path, relative: PathBuf) -> Result<Entry> {
    let metadata = parent.symlink_metadata(name)?;
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 && !metadata.file_type().is_symlink() {
            bail!("An unsupported reparse point is outside the cleanup scope.");
        }
    }
    let stamp = stamp(&metadata)?;
    let kind = if metadata.file_type().is_symlink() {
        Kind::Link
    } else if metadata.is_dir() {
        Kind::Directory
    } else if metadata.is_file() {
        Kind::File
    } else {
        bail!("Unsupported filesystem object; cleanup is unavailable.");
    };
    let mut hash = Sha256::new();
    hash.update(&stamp);
    let mut bytes = 0;
    let identity;
    if kind == Kind::File {
        let mut file = parent.open(name)?;
        #[cfg(unix)]
        {
            use cap_std::fs::MetadataExt;
            let metadata = file.metadata()?;
            identity = [metadata.dev().to_le_bytes(), metadata.ino().to_le_bytes()].concat();
        }
        #[cfg(target_os = "linux")]
        {
            let parent_mount = mount_id(&parent.try_clone()?.into_std_file())?;
            let file_mount = mount_id(&file.try_clone()?.into_std())?;
            if parent_mount != file_mount {
                bail!("A mounted file is outside the cleanup scope.");
            }
        }
        #[cfg(windows)]
        {
            identity = windows_identity(&file.try_clone()?.into_std())?;
        }
        if stamp != self::stamp(&file.metadata()?)? {
            bail!("A file changed during inspection; request a new preview.");
        }
        let mut buffer = [0; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            bytes += count as u64;
            if bytes > metadata.len() {
                bail!("A file grew during inspection; request a new preview.");
            }
            hash.update(&buffer[..count]);
        }
        if bytes != metadata.len() || stamp != self::stamp(&file.metadata()?)? {
            bail!("A file changed during inspection; request a new preview.");
        }
    } else if kind == Kind::Directory {
        identity = self::identity(&parent.open_dir(name)?)?;
    } else {
        let target = parent.read_link_contents(name)?;
        hash.update(target.as_os_str().as_encoded_bytes());
        #[cfg(unix)]
        {
            identity = stamp.clone();
        }
        #[cfg(windows)]
        {
            use cap_std::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
            };
            let mut options = cap_std::fs::OpenOptions::new();
            options
                .access_mode(FILE_READ_ATTRIBUTES)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS);
            identity = windows_identity(&parent.open_with(name, &options)?.into_std())?;
        }
    }
    hash.update(&identity);
    if stamp != self::stamp(&parent.symlink_metadata(name)?)? {
        bail!("A filesystem entry changed during inspection; request a new preview.");
    }
    Ok(Entry {
        path: relative,
        kind,
        bytes,
        #[cfg(windows)]
        stamp,
        hash: hash.finalize().to_vec(),
        identity,
    })
}

pub(super) fn scan(dir: &Dir) -> Result<Vec<Entry>> {
    fn visit(
        dir: &Dir,
        prefix: &Path,
        root: &Metadata,
        result: &mut Vec<Entry>,
        depth: usize,
    ) -> Result<()> {
        if depth > 64 {
            bail!("Directory nesting exceeds the maintenance limit.");
        }
        let mut names = Vec::new();
        for item in dir.entries()? {
            if names.len() + result.len() >= 100_000 {
                bail!("File count exceeds the maintenance limit.");
            }
            names.push(item?.file_name());
        }
        names.sort();
        for name in names {
            if result.len() >= 100_000 {
                bail!("File count exceeds the maintenance limit.");
            }
            let path = Path::new(&name);
            let value = entry(dir, path, prefix.join(path))?;
            if value.kind == Kind::Directory {
                let metadata = dir.symlink_metadata(path)?;
                if !same_device(root, &metadata) {
                    bail!("A nested mount is outside the cleanup scope.");
                }
                let child = dir.open_dir(path)?;
                same_mount(dir, &child)?;
                if stamp(&metadata)? != stamp(&child.dir_metadata()?)? {
                    bail!("A directory changed during inspection; request a new preview.");
                }
                visit(&child, &prefix.join(path), root, result, depth + 1)?;
            }
            result.push(value);
        }
        Ok(())
    }
    let mut result = Vec::new();
    visit(dir, Path::new(""), &dir.dir_metadata()?, &mut result, 0)
        .context("Local data could not be inspected safely")?;
    Ok(result)
}

pub(super) fn digest(entries: &[Entry]) -> Vec<u8> {
    let mut hash = Sha256::new();
    for entry in entries {
        let name = entry.path.as_os_str().as_encoded_bytes();
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name);
        hash.update(&entry.hash);
    }
    hash.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_files_are_read_before_their_directories_and_mutation_changes_the_digest() {
        let path = std::env::temp_dir().join(format!("am-claude-tree-{}", crate::store::uuid_v4()));
        std::fs::create_dir_all(path.join("projects/session")).unwrap();
        std::fs::write(
            path.join("projects/session/data.jsonl"),
            b"PRIVATE_SESSION_SENTINEL",
        )
        .unwrap();
        let dir = Dir::open_ambient_dir(&path, cap_std::ambient_authority()).unwrap();
        let first = scan(&dir).unwrap();
        assert_eq!(first[0].path, Path::new("projects/session/data.jsonl"));
        assert_eq!(first.last().unwrap().path, Path::new("projects"));
        assert_eq!(first.iter().map(|e| e.bytes).sum::<u64>(), 24);
        assert_eq!(digest(&first), digest(&scan(&dir).unwrap()));
        std::fs::write(path.join("projects/session/data.jsonl"), b"NEW_SESSION").unwrap();
        assert_ne!(digest(&first), digest(&scan(&dir).unwrap()));
        drop(dir);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn external_symlinks_are_only_described_and_never_followed() {
        let path = std::env::temp_dir().join(format!("am-claude-link-{}", crate::store::uuid_v4()));
        std::fs::create_dir_all(path.join("data")).unwrap();
        std::fs::write(path.join("outside"), b"EXTERNAL_SOURCE_SENTINEL").unwrap();
        std::os::unix::fs::symlink(path.join("outside"), path.join("data/link")).unwrap();
        let dir = Dir::open_ambient_dir(path.join("data"), cap_std::ambient_authority()).unwrap();
        let entries = scan(&dir).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, Kind::Link);
        assert_eq!(entries[0].bytes, 0);
        assert_eq!(
            std::fs::read(path.join("outside")).unwrap(),
            b"EXTERNAL_SOURCE_SENTINEL"
        );
        drop(dir);
        std::fs::remove_dir_all(path).unwrap();
    }
}
