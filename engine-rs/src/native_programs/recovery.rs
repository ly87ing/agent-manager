//! Ordinary failure recovery never removes an occupied installation to restore a backup.
use anyhow::{bail, Context as _, Result};
use std::path::{Path, PathBuf};

/// Unlike `rename`, this must also refuse a destination created after preflight.
pub(super) fn move_absent(from: &Path, to: &Path) -> Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let result = {
        use std::os::unix::ffi::OsStrExt;
        let from = std::ffi::CString::new(from.as_os_str().as_bytes())?;
        let to = std::ffi::CString::new(to.as_os_str().as_bytes())?;
        #[cfg(target_os = "macos")]
        let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        result == 0
    };
    #[cfg(windows)]
    let result = {
        use std::os::windows::ffi::OsStrExt;
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(from.as_ptr(), to.as_ptr(), 0) != 0
        }
    };
    if !result {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!(
                "Could not move {} to {}; no destination was replaced. Inspect both locations",
                from.display(),
                to.display()
            )
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("am-program-recovery-{}", crate::store::uuid_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn moves_never_replace_files_directories_or_dangling_links() {
        let root = Fixture::new();
        let source = root.0.join("source");
        let target = root.0.join("target");
        std::fs::write(&source, b"backup").unwrap();
        std::fs::write(&target, b"external").unwrap();
        assert!(move_absent(&source, &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"external");
        std::fs::remove_file(&target).unwrap();
        std::fs::create_dir(&target).unwrap();
        assert!(move_absent(&source, &target).is_err());
        std::fs::remove_dir(&target).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("missing", &target).unwrap();
            assert!(move_absent(&source, &target).is_err());
            assert_eq!(std::fs::read_link(&target).unwrap(), Path::new("missing"));
            std::fs::remove_file(&target).unwrap();
        }
        move_absent(&source, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"backup");
    }

    #[test]
    fn missing_backups_and_occupied_destinations_report_partial_recovery() {
        let root = Fixture::new();
        let original = root.0.join("original");
        let backup = root.0.join("backup");
        std::fs::write(&original, b"old").unwrap();
        let mut journal = Journal::default();
        journal.move_entry(&original, &backup).unwrap();
        std::fs::write(&original, b"external").unwrap();
        assert_eq!(journal.rollback(|_| Ok(())).len(), 1);
        assert_eq!(std::fs::read(&original).unwrap(), b"external");
        assert_eq!(std::fs::read(&backup).unwrap(), b"old");
        std::fs::remove_file(&original).unwrap();
        std::fs::remove_file(&backup).unwrap();
        assert_eq!(journal.rollback(|_| Ok(())).len(), 1);
    }

    #[test]
    fn changed_backup_bytes_are_retained_instead_of_restored() {
        let root = Fixture::new();
        let original = root.0.join("original");
        let backup = root.0.join("backup");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("entry"), b"original bytes").unwrap();
        let mut journal = Journal::default();
        journal.move_entry(&original, &backup).unwrap();
        std::fs::write(backup.join("entry"), b"external bytes").unwrap();
        assert_eq!(journal.rollback(|_| Ok(())).len(), 1);
        assert!(!original.exists());
        assert_eq!(
            std::fs::read(backup.join("entry")).unwrap(),
            b"external bytes"
        );
    }

    #[test]
    fn current_is_restored_after_later_failure_and_external_changes_are_retained() {
        let root = Fixture::new();
        for name in ["old", "new", "external"] {
            std::fs::create_dir(root.0.join(name)).unwrap();
        }
        let current = root.0.join("current");
        crate::platform::link_dir(&root.0.join("old"), &current).unwrap();
        let mut journal = Journal::default();
        journal
            .replace(&current, &root.0.join("new"), false)
            .unwrap();
        assert!(journal.rollback(|_| Ok(())).is_empty());
        assert_eq!(
            current.canonicalize().unwrap(),
            root.0.join("old").canonicalize().unwrap()
        );
        let mut journal = Journal::default();
        journal
            .replace(&current, &root.0.join("new"), false)
            .unwrap();
        crate::platform::remove_link(&current).unwrap();
        crate::platform::link_dir(&root.0.join("external"), &current).unwrap();
        assert_eq!(journal.rollback(|_| Ok(())).len(), 1);
        assert_eq!(
            current.canonicalize().unwrap(),
            root.0.join("external").canonicalize().unwrap()
        );
        assert!(std::fs::read_dir(&root.0)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".am-recovery-")
                && entry.path().canonicalize().ok() == root.0.join("old").canonicalize().ok()));
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Pointer {
    Missing,
    Link(PathBuf),
    Shim(Vec<u8>),
}

impl Pointer {
    fn read(path: &Path) -> Result<Self> {
        let meta = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::Missing),
            Err(error) => return Err(error.into()),
        };
        if meta.file_type().is_symlink() {
            return Ok(Self::Link(std::fs::read_link(path)?));
        }
        if cfg!(windows) && meta.is_file() && meta.len() <= 64 * 1024 {
            use crate::platform::Canon;
            use std::io::Read;
            let parent =
                super::uv_metadata_files::Directory::open(&path.parent().unwrap().canon()?)?;
            let mut bytes = Vec::new();
            parent
                .file(path.file_name().unwrap())?
                .take(64 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() <= 64 * 1024 {
                return Ok(Self::Shim(bytes));
            }
        }
        bail!("{} is not an owned command pointer", path.display())
    }
}

enum Undo {
    Move {
        from: PathBuf,
        to: PathBuf,
        fingerprint: Vec<u8>,
    },
    Pointer {
        path: PathBuf,
        previous: PathBuf,
        before: Pointer,
        written: Pointer,
    },
}

#[derive(Default)]
pub(super) struct Journal {
    undo: Vec<Undo>,
}

fn sibling(path: &Path) -> PathBuf {
    path.with_file_name(format!(".am-recovery-{}", crate::store::uuid_v4()))
}

impl Journal {
    pub(super) fn move_entry(&mut self, from: &Path, to: &Path) -> Result<()> {
        let fingerprint = super::recovery_snapshot::fingerprint(from)?;
        move_absent(from, to)?;
        self.undo.push(Undo::Move {
            from: from.into(),
            to: to.into(),
            fingerprint: fingerprint.clone(),
        });
        if super::recovery_snapshot::fingerprint(to)? != fingerprint {
            bail!(
                "Moved program contents changed at {}; recovery evidence is uncertain",
                to.display()
            );
        }
        Ok(())
    }

    /// Retain the previous pointer before publication, including a failing Windows replacement.
    pub(super) fn replace(&mut self, path: &Path, target: &Path, shim: bool) -> Result<()> {
        let before = Pointer::read(path)?;
        if !shim && matches!(before, Pointer::Shim(_)) {
            bail!("{} is not a directory pointer", path.display());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let stage = sibling(path);
        if shim {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stage)?;
            file.write_all(crate::platform::cmd_shim_text(target)?.as_bytes())?;
        } else {
            let resolved = path.parent().unwrap_or(Path::new(".")).join(target);
            if resolved.is_dir() {
                crate::platform::link_dir(target, &stage)?;
            } else {
                crate::platform::link_file(target, &stage)?;
            }
        }
        let written = Pointer::read(&stage)?;
        let previous = sibling(path);
        if before != Pointer::Missing {
            if Pointer::read(path)? != before {
                bail!(
                    "Command pointer changed before replacement: {}",
                    path.display()
                );
            }
            move_absent(path, &previous)?;
        }
        // Journal before publishing, so a failure after moving the old pointer is recoverable.
        self.undo.push(Undo::Pointer {
            path: path.into(),
            previous,
            before,
            written,
        });
        move_absent(&stage, path)?;
        Ok(())
    }

    pub(super) fn rollback(&self, mut validate: impl FnMut(&Path) -> Result<()>) -> Vec<String> {
        let mut failures = Vec::new();
        for undo in self.undo.iter().rev() {
            let result = (|| -> Result<()> {
                match undo {
                    Undo::Move {
                        from,
                        to,
                        fingerprint,
                    } => {
                        validate(from)?;
                        validate(to)?;
                        if &super::recovery_snapshot::fingerprint(to)? != fingerprint {
                            bail!(
                                "Moved program contents changed; retained at {}",
                                to.display()
                            );
                        }
                        move_absent(to, from)?;
                    }
                    Undo::Pointer {
                        path,
                        previous,
                        before,
                        written,
                    } => {
                        validate(path)?;
                        validate(previous)?;
                        if &Pointer::read(previous)? != before {
                            bail!(
                                "Previous pointer changed or missing at {}",
                                previous.display()
                            );
                        }
                        let actual = Pointer::read(path)?;
                        if actual != Pointer::Missing {
                            if &actual != written {
                                bail!(
                                    "Changed pointer retained at {}; previous pointer at {}",
                                    path.display(),
                                    previous.display()
                                );
                            }
                            // Keep the displaced pointer too; recovery never deletes conflicting bytes.
                            move_absent(path, &sibling(path))?;
                        }
                        if before != &Pointer::Missing {
                            move_absent(previous, path)?;
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                failures.push(format!("{error:#}"));
                // Earlier moves can feed this pointer. Retain them after an uncertain recovery.
                break;
            }
        }
        failures
    }

    pub(super) fn finish(&self) -> Result<()> {
        for undo in &self.undo {
            if let Undo::Pointer {
                previous, before, ..
            } = undo
            {
                if &Pointer::read(previous)? != before {
                    bail!(
                        "Previous pointer changed or missing at {}",
                        previous.display()
                    );
                }
                if before != &Pointer::Missing {
                    crate::platform::remove_link(previous).with_context(|| {
                        format!("Previous pointer retained at {}", previous.display())
                    })?;
                }
            }
        }
        Ok(())
    }
}
