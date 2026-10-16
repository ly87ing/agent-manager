//! Fixed native NTP file publication, under the cooperative host-directory lock.
use super::repair_linux_ntp_config as config;
use serde::{Deserialize, Serialize};
use std::{
    ffi::CString,
    fs::File,
    io::Write,
    os::fd::{AsRawFd, FromRawFd},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const DIRECTORY: &str = "etc/systemd/timesyncd.conf.d";

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Publication {
    pub directory: Option<config::Identity>,
    pub file: Option<config::Content>,
    pub durable: bool,
    pub status: String,
}

impl Publication {
    pub(super) fn project(&self, before: &config::State, server: &str) -> Option<config::State> {
        if !matches!(
            self.status.as_str(),
            "saved" | "unchanged" | "stale" | "failed" | "outcome_unknown"
        ) {
            return None;
        }
        let mut after = before.clone();
        if let Some(directory) = &self.directory {
            after = after.with_directory(directory.clone())?;
        }
        if let Some(file) = &self.file {
            after = after.with_file(file.clone(), server)?;
        }
        if self.status == "saved" && self.file.is_none() {
            return None;
        }
        if self.status == "unchanged"
            && (self.directory.is_some()
                || self.file.is_some()
                || before.saved.as_deref() != Some(server))
        {
            return None;
        }
        if self.durable != matches!(self.status.as_str(), "saved" | "unchanged") {
            return None;
        }
        Some(after)
    }
}

pub(super) struct HostLock {
    anchor: PathBuf,
    uid: u32,
    parent: File,
}

fn open_child(parent: &File, name: &str, flags: i32) -> Option<File> {
    let name = CString::new(name).ok()?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            0o600 as libc::c_uint,
        )
    };
    (fd >= 0).then(|| unsafe { File::from_raw_fd(fd) })
}

fn same_entry(parent: &File, name: &str, file: &File) -> bool {
    let Ok(name) = CString::new(name) else {
        return false;
    };
    let mut meta = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            meta.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return false;
    }
    let a = unsafe { meta.assume_init() };
    let Ok(b) = file.metadata() else {
        return false;
    };
    a.st_mode & libc::S_IFMT == libc::S_IFREG
        && b.is_file()
        && a.st_dev as u64 == b.dev()
        && a.st_ino as u64 == b.ino()
        && a.st_uid == b.uid()
        && a.st_nlink == 1
        && b.nlink() == 1
}

struct Stage {
    parent: File,
    name: String,
    file: File,
}
impl Stage {
    fn create(parent: &File, uid: u32) -> Option<Self> {
        let parent = parent.try_clone().ok()?;
        let name = format!(".agent-manager-{}.tmp", crate::store::uuid_v4());
        let file = open_child(&parent, &name, libc::O_RDWR | libc::O_CREAT | libc::O_EXCL)?;
        let stage = Self { parent, name, file };
        if unsafe { libc::fchmod(stage.file.as_raw_fd(), 0o600) } != 0 {
            return None;
        }
        let meta = stage.file.metadata().ok()?;
        (meta.is_file() && meta.uid() == uid && meta.nlink() == 1 && meta.mode() & 0o777 == 0o600)
            .then_some(stage)
    }

    fn publish(&self, replace: bool) -> bool {
        if !same_entry(&self.parent, &self.name, &self.file) {
            return false;
        }
        let old = CString::new(self.name.as_str()).unwrap();
        let new = CString::new(config::NAME).unwrap();
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                self.parent.as_raw_fd(),
                old.as_ptr(),
                self.parent.as_raw_fd(),
                new.as_ptr(),
                if replace { 0 } else { libc::RENAME_NOREPLACE },
            )
        };
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::renameatx_np(
                self.parent.as_raw_fd(),
                old.as_ptr(),
                self.parent.as_raw_fd(),
                new.as_ptr(),
                if replace { 0 } else { libc::RENAME_EXCL },
            )
        };
        result == 0
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        if same_entry(&self.parent, &self.name, &self.file) {
            if let Ok(name) = CString::new(self.name.as_str()) {
                let _ = unsafe { libc::unlinkat(self.parent.as_raw_fd(), name.as_ptr(), 0) };
            }
        }
    }
}

enum Creation {
    Failed,
    Unknown(Option<config::Identity>),
    Ready(File, config::Identity),
}

impl HostLock {
    #[cfg(target_os = "linux")]
    pub(super) fn acquire(before: &config::State) -> Option<Self> {
        if unsafe { libc::getuid() } != 0 || unsafe { libc::geteuid() } != 0 {
            return None;
        }
        Self::at(Path::new("/"), 0, before)
    }

    fn at(anchor: &Path, uid: u32, before: &config::State) -> Option<Self> {
        let parent = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(anchor.join("etc/systemd"))
            .ok()?;
        if Some(config::record_directory(&parent, uid)?)
            != before.directories.get("etc/systemd")?.clone()
        {
            return None;
        }
        fs2::FileExt::try_lock_exclusive(&parent).ok()?;
        let lock = Self {
            anchor: anchor.into(),
            uid,
            parent,
        };
        lock.fresh(before).then_some(lock)
    }

    fn fresh(&self, expected: &config::State) -> bool {
        config::record_directory(&self.parent, self.uid).as_ref()
            == expected
                .directories
                .get("etc/systemd")
                .and_then(Option::as_ref)
            && config::read_at(&self.anchor, self.uid).as_ref() == Some(expected)
    }

    fn create_directory(&self, gid: u32) -> Creation {
        let name = CString::new("timesyncd.conf.d").unwrap();
        if unsafe { libc::mkdirat(self.parent.as_raw_fd(), name.as_ptr(), 0o755) } != 0 {
            return Creation::Failed;
        }
        let Some(directory) = open_child(
            &self.parent,
            "timesyncd.conf.d",
            libc::O_RDONLY | libc::O_DIRECTORY,
        ) else {
            return Creation::Unknown(None);
        };
        if unsafe { libc::fchown(directory.as_raw_fd(), self.uid, gid) } != 0
            || unsafe { libc::fchmod(directory.as_raw_fd(), 0o755) } != 0
        {
            return Creation::Unknown(config::record_directory(&directory, self.uid));
        }
        let Some(identity) = config::record_directory(&directory, self.uid) else {
            return Creation::Unknown(None);
        };
        Creation::Ready(directory, identity)
    }

    pub(super) fn publish(
        &self,
        before: &config::State,
        server: &str,
        ready: impl Fn(&config::State) -> bool,
    ) -> Publication {
        let mut result = Publication {
            status: "stale".into(),
            ..Default::default()
        };
        let Some(text) = config::generated(server) else {
            return result;
        };
        if !self.fresh(before) || !ready(before) {
            return result;
        }
        if before.saved.as_deref()
            == Some(server.trim_end_matches('.').to_ascii_lowercase().as_str())
        {
            result.status = "unchanged".into();
            result.durable = true;
            return result;
        }
        let Some((mode, gid)) = before.file_policy() else {
            return result;
        };
        let mut expected = before.clone();
        result.status = "failed".into();
        let directory = if before.directory_missing() {
            let (directory, identity) = match self.create_directory(gid) {
                Creation::Ready(directory, identity) => (directory, identity),
                Creation::Unknown(identity) => {
                    result.directory = identity;
                    result.status = "outcome_unknown".into();
                    return result;
                }
                Creation::Failed => return result,
            };
            result.directory = Some(identity.clone());
            let Some(after) = before.with_directory(identity) else {
                return result;
            };
            expected = after;
            if self.parent.sync_all().is_err() {
                result.status = "outcome_unknown".into();
                return result;
            }
            directory
        } else {
            let Some(directory) = open_child(
                &self.parent,
                "timesyncd.conf.d",
                libc::O_RDONLY | libc::O_DIRECTORY,
            ) else {
                return result;
            };
            directory
        };
        if config::record_directory(&directory, self.uid).as_ref()
            != expected.directories.get(DIRECTORY).and_then(Option::as_ref)
            || !self.fresh(&expected)
            || !ready(&expected)
        {
            result.status = "stale".into();
            return result;
        }
        let Some(mut stage) = Stage::create(&directory, self.uid) else {
            return result;
        };
        if unsafe { libc::fchown(stage.file.as_raw_fd(), self.uid, gid) } != 0
            || stage.file.write_all(text.as_bytes()).is_err()
            || unsafe { libc::fchmod(stage.file.as_raw_fd(), mode as libc::mode_t) } != 0
            || stage.file.sync_all().is_err()
        {
            return result;
        }
        let Some(receipt) = config::record_file(&stage.file, self.uid, &text) else {
            return result;
        };
        let Some(after) = expected.with_file(receipt.clone(), server) else {
            return result;
        };
        if !self.fresh(&expected) || !ready(&expected) {
            result.status = "stale".into();
            return result;
        }
        let replace = before
            .files
            .get(&format!("{DIRECTORY}/{}", config::NAME))
            .is_some_and(Option::is_some);
        if !stage.publish(replace) {
            return result;
        }
        result.file = Some(receipt);
        result.status = "outcome_unknown".into();
        if directory.sync_all().is_err() || !self.fresh(&after) {
            return result;
        }
        result.durable = true;
        result.status = "saved".into();
        result
    }
}

#[cfg(test)]
#[path = "repair_linux_ntp_file_tests.rs"]
mod tests;
