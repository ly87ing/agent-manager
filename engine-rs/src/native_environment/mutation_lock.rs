//! One confirmed environment mutation per OS user, independent of app data directories.
use anyhow::{bail, Result};

#[cfg(target_os = "linux")]
pub(super) struct Guard {
    _file: std::fs::File,
}
#[cfg(target_os = "linux")]
pub(super) fn acquire() -> Result<Guard> {
    use fs2::FileExt;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let uid = unsafe { libc::geteuid() };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(format!(
            "/tmp/io.github.ly87ing.agent-manager.environment-{uid}.lock"
        ))?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.uid() != uid || meta.mode() & 0o077 != 0 || meta.nlink() != 1 {
        bail!("The environment repair lock is not a private regular file.");
    }
    file.try_lock_exclusive().map_err(|_| {
        anyhow::anyhow!(
            "An environment repair or time correction is already running for this user."
        )
    })?;
    Ok(Guard { _file: file })
}

#[cfg(windows)]
pub(super) struct Guard {
    handle: windows_sys::Win32::Foundation::HANDLE,
}
#[cfg(windows)]
impl Drop for Guard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.handle);
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}
#[cfg(windows)]
pub(super) fn acquire() -> Result<Guard> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS},
        System::Threading::CreateMutexW,
    };
    let sid =
        super::windows::script("[Security.Principal.WindowsIdentity]::GetCurrent().User.Value")
            .filter(|sid| {
                sid.starts_with("S-1-")
                    && sid.len() <= 184
                    && sid[1..].bytes().all(|ch| ch.is_ascii_digit() || ch == b'-')
            })
            .ok_or_else(|| anyhow::anyhow!("Cannot identify the current user's mutation lock."))?;
    let name: Vec<u16> = format!("Global\\io.github.ly87ing.agent-manager.environment-{sid}\0")
        .encode_utf16()
        .collect();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
    if handle.is_null() {
        bail!("Cannot acquire the environment mutation lock.");
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            CloseHandle(handle);
        }
        bail!("An environment repair or time correction is already running for this user.");
    }
    Ok(Guard { handle })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_mutation_lock_is_exclusive_across_open_handles_and_released_after_drop() {
        const CHILD: &str = "AM_NATIVE_MUTATION_LOCK_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "native_environment::mutation_lock::tests::native_mutation_lock_is_exclusive_across_open_handles_and_released_after_drop", "--nocapture"])
                .env(CHILD, "1").output().unwrap();
            assert!(output.status.success(), "{output:?}");
            return;
        }
        let first = acquire().unwrap();
        assert!(acquire().is_err());
        drop(first);
        acquire().expect("The mutation lock must be available after drop");
    }
}
