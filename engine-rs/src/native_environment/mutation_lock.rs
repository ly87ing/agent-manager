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
fn user_sid() -> Result<String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, IsValidSid, TokenUser,
            TOKEN_QUERY, TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let result = (|| {
            let mut needed = 0;
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
            if needed < std::mem::size_of::<TOKEN_USER>() as u32 || needed > 64 * 1024 {
                bail!("Cannot read the current user's mutation-lock identity.");
            }
            let mut bytes = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
            if GetTokenInformation(
                token,
                TokenUser,
                bytes.as_mut_ptr().cast(),
                needed,
                &mut needed,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            let sid = (*(bytes.as_ptr().cast::<TOKEN_USER>())).User.Sid;
            if IsValidSid(sid) == 0 {
                bail!("The current user's mutation-lock SID is invalid.");
            }
            let mut text = std::ptr::null_mut();
            if ConvertSidToStringSidW(sid, &mut text) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let value = (0..=184)
                .find(|index| *text.add(*index) == 0)
                .ok_or_else(|| anyhow::anyhow!("The mutation-lock SID is too long."))
                .and_then(|length| {
                    Ok(String::from_utf16(std::slice::from_raw_parts(
                        text, length,
                    ))?)
                });
            LocalFree(text.cast());
            value
        })();
        CloseHandle(token);
        result
    }
}

#[cfg(windows)]
pub(super) fn acquire() -> Result<Guard> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS},
        System::Threading::CreateMutexW,
    };
    let sid = user_sid()?;
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
        if std::env::var_os(CHILD).as_deref() == Some(std::ffi::OsStr::new("held")) {
            let error = acquire()
                .err()
                .expect("Another process owns the mutation lock");
            assert_eq!(
                error.to_string(),
                "An environment repair or time correction is already running for this user."
            );
            return;
        }
        if std::env::var_os(CHILD).is_none() {
            #[cfg(windows)]
            {
                use windows_sys::Win32::{
                    Foundation::{GetLastError, ERROR_ALREADY_EXISTS},
                    System::{SystemInformation::GetSystemDirectoryW, Threading::CreateMutexW},
                };
                let mut directory = [0u16; 32768];
                let length =
                    unsafe { GetSystemDirectoryW(directory.as_mut_ptr(), directory.len() as u32) }
                        as usize;
                assert!(length > 0 && length < directory.len());
                let whoami =
                    std::path::PathBuf::from(String::from_utf16(&directory[..length]).unwrap())
                        .join("whoami.exe");
                let reference = super::super::command::bounded(
                    whoami.to_str().unwrap(),
                    &["/user", "/fo", "csv", "/nh"],
                    std::time::Duration::from_secs(8),
                )
                .unwrap();
                let sid = reference
                    .rsplit(',')
                    .next()
                    .unwrap()
                    .trim()
                    .trim_matches('"');
                assert_eq!(user_sid().unwrap(), sid);
                // An independently named old-format mutex must exclude the new
                // native reader; switching identity sources cannot split locks.
                let name: Vec<_> =
                    format!("Global\\io.github.ly87ing.agent-manager.environment-{sid}\0")
                        .encode_utf16()
                        .collect();
                let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
                assert!(!handle.is_null());
                let legacy = Guard { handle };
                assert_ne!(unsafe { GetLastError() }, ERROR_ALREADY_EXISTS);
                let error = acquire()
                    .err()
                    .expect("The legacy mutex must exclude this process");
                assert_eq!(
                    error.to_string(),
                    "An environment repair or time correction is already running for this user."
                );
                drop(legacy);
            }
            let holder = acquire().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "native_environment::mutation_lock::tests::native_mutation_lock_is_exclusive_across_open_handles_and_released_after_drop", "--nocapture"])
                .env(CHILD, "held").env("HOME", "YOUR_UNRELATED_HOME").env("USERPROFILE", "YOUR_UNRELATED_PROFILE")
                .output().unwrap();
            assert!(output.status.success(), "{output:?}");
            drop(holder);
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "native_environment::mutation_lock::tests::native_mutation_lock_is_exclusive_across_open_handles_and_released_after_drop", "--nocapture"])
                .env(CHILD, "1").env("HOME", "YOUR_UNRELATED_HOME").env("USERPROFILE", "YOUR_UNRELATED_PROFILE")
                .output().unwrap();
            assert!(output.status.success(), "{output:?}");
            return;
        }
        let first = acquire().unwrap();
        assert!(acquire().is_err());
        drop(first);
        acquire().expect("The mutation lock must be available after drop");
    }
}
