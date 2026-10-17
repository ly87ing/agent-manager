//! Read-only ordinary objects, scoped to held directories rather than unchecked paths.
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::path::{Component, Path};

pub(super) struct Directory {
    file: File,
    #[cfg(windows)]
    path: std::path::PathBuf,
    #[cfg(windows)]
    ancestors: Vec<File>,
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Expected an ordinary metadata object",
    )
}

impl Directory {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() || path.components().count() > 128 {
            return Err(invalid());
        }
        let mut components = path.components();
        #[cfg(unix)]
        let mut directory = {
            use std::os::unix::fs::OpenOptionsExt;
            if components.next() != Some(Component::RootDir) {
                return Err(invalid());
            }
            Self {
                file: std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open("/")?,
            }
        };
        #[cfg(windows)]
        let mut directory = {
            use std::path::Prefix;
            let Some(Component::Prefix(prefix)) = components.next() else {
                return Err(invalid());
            };
            if !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
                || components.next() != Some(Component::RootDir)
            {
                return Err(invalid());
            }
            let root = Path::new(prefix.as_os_str()).join("\\");
            Self {
                file: windows_open(&root, true)?,
                path: root,
                ancestors: Vec::new(),
            }
        };
        for component in components {
            let Component::Normal(name) = component else {
                return Err(invalid());
            };
            directory = directory.child(name)?;
        }
        Ok(directory)
    }

    pub(super) fn child(&self, name: &OsStr) -> io::Result<Self> {
        if Path::new(name).components().count() != 1
            || !matches!(
                Path::new(name).components().next(),
                Some(Component::Normal(_))
            )
        {
            return Err(invalid());
        }
        #[cfg(unix)]
        {
            Ok(Self {
                file: unix_open(&self.file, name, true)?,
            })
        }
        #[cfg(windows)]
        {
            // Retain ancestor handles that deny write/delete opens during path-based reads.
            let mut ancestors = self
                .ancestors
                .iter()
                .map(File::try_clone)
                .collect::<io::Result<Vec<_>>>()?;
            ancestors.push(self.file.try_clone()?);
            let path = self.path.join(name);
            Ok(Self {
                file: windows_open(&path, true)?,
                path,
                ancestors,
            })
        }
    }

    pub(super) fn file(&self, name: &OsStr) -> io::Result<File> {
        #[cfg(unix)]
        {
            unix_open(&self.file, name, false)
        }
        #[cfg(windows)]
        {
            windows_open(&self.path.join(name), false)
        }
    }

    pub(super) fn entries(&self, limit: usize) -> io::Result<Vec<OsString>> {
        #[cfg(unix)]
        {
            unix_entries(&self.file, limit)
        }
        #[cfg(windows)]
        {
            let mut names = Vec::new();
            for entry in std::fs::read_dir(&self.path)? {
                if names.len() == limit {
                    return Err(invalid());
                }
                names.push(entry?.file_name());
            }
            Ok(names)
        }
    }
}

#[cfg(unix)]
fn unix_open(parent: &File, name: &OsStr, directory: bool) -> io::Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| invalid())?;
    let flags = libc::O_RDONLY
        | libc::O_NOFOLLOW
        | libc::O_CLOEXEC
        | libc::O_NONBLOCK
        | if directory { libc::O_DIRECTORY } else { 0 };
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata()?;
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err(invalid());
    }
    Ok(file)
}

#[cfg(unix)]
fn unix_entries(parent: &File, limit: usize) -> io::Result<Vec<OsString>> {
    use std::os::fd::IntoRawFd;
    use std::os::unix::ffi::OsStringExt;
    // A fresh descriptor gives each enumeration an independent directory offset.
    let fd = unix_open(parent, OsStr::new("."), true)?.into_raw_fd();
    let stream = unsafe { libc::fdopendir(fd) };
    if stream.is_null() {
        let error = io::Error::last_os_error();
        unsafe {
            libc::close(fd);
        }
        return Err(error);
    }
    struct Stream(*mut libc::DIR);
    impl Drop for Stream {
        fn drop(&mut self) {
            unsafe {
                libc::closedir(self.0);
            }
        }
    }
    let stream = Stream(stream);
    let mut names = Vec::new();
    loop {
        #[cfg(target_os = "macos")]
        let errno = unsafe { libc::__error() };
        #[cfg(target_os = "linux")]
        let errno = unsafe { libc::__errno_location() };
        unsafe {
            *errno = 0;
        }
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            return if unsafe { *errno } == 0 {
                Ok(names)
            } else {
                Err(io::Error::last_os_error())
            };
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if matches!(name, b"." | b"..") {
            continue;
        }
        if names.len() == limit {
            return Err(invalid());
        }
        names.push(OsString::from_vec(name.to_vec()));
    }
}

#[cfg(windows)]
fn windows_open(path: &Path, directory: bool) -> io::Result<File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::*;
    let mut options = std::fs::OpenOptions::new();
    options
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    if directory {
        options.access_mode(FILE_READ_ATTRIBUTES);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(invalid());
    }
    Ok(file)
}
