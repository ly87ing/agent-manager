//! Match a running Windows image to the selected file, not merely its basename.
use anyhow::{bail, Result};
use std::{
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, ERROR_NO_MORE_FILES, HANDLE,
        INVALID_HANDLE_VALUE, STILL_ACTIVE,
    },
    Globalization::{CompareStringOrdinal, CSTR_EQUAL},
    Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO, FILE_READ_ATTRIBUTES,
    },
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        Threading::{
            GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn file_id(path: &Path) -> std::io::Result<(u64, [u8; 16])> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .access_mode(FILE_READ_ATTRIBUTES)
        .open(path)?;
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
        return Err(std::io::Error::last_os_error());
    }
    let info = unsafe { info.assume_init() };
    Ok((info.VolumeSerialNumber, info.FileId.Identifier))
}

fn exited(process: HANDLE) -> bool {
    let mut code = STILL_ACTIVE as u32;
    unsafe { GetExitCodeProcess(process, &mut code) != 0 && code != STILL_ACTIVE as u32 }
}

pub(super) fn already_running(target: &Path) -> Result<bool> {
    let selected = file_id(target)?;
    let name: Vec<_> = target
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Missing executable name"))?
        .encode_wide()
        .collect();
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        bail!("Unable to check running applications");
    }
    let snapshot = Handle(snapshot);
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut found = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
    let mut count = 0;
    while found {
        count += 1;
        if count > 8192 {
            bail!("Too many processes to verify the selected application safely");
        }
        let length = entry
            .szExeFile
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(entry.szExeFile.len());
        let same_name = unsafe {
            CompareStringOrdinal(
                name.as_ptr(),
                name.len() as i32,
                entry.szExeFile.as_ptr(),
                length as i32,
                1,
            )
        };
        if same_name == 0 {
            bail!("Unable to compare running application names");
        }
        {
            let process =
                unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID) };
            if process.is_null() {
                if same_name == CSTR_EQUAL && unsafe { GetLastError() } != ERROR_INVALID_PARAMETER {
                    bail!("Unable to verify a matching running application");
                }
            } else {
                let process = Handle(process);
                let mut path = [0u16; 32768];
                let mut length = path.len() as u32;
                let image = if unsafe {
                    QueryFullProcessImageNameW(process.0, 0, path.as_mut_ptr(), &mut length)
                } != 0
                {
                    use std::os::windows::ffi::OsStringExt;
                    file_id(Path::new(&std::ffi::OsString::from_wide(
                        &path[..length as usize],
                    )))
                } else {
                    Err(std::io::Error::last_os_error())
                };
                match image {
                    Ok(id) if id == selected => return Ok(true),
                    Ok(_) => {}
                    Err(_) if exited(process.0) => {}
                    Err(_) if same_name == CSTR_EQUAL => {
                        bail!("Unable to verify a matching running application")
                    }
                    Err(_) => {}
                }
            }
        }
        found = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
    }
    if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
        bail!("Unable to complete the running application check");
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_process_helper() {
        let Some(marker) = std::env::var_os("AM_LAUNCH_IMAGE_HELPER") else {
            return;
        };
        std::fs::write(&marker, b"ready").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    #[test]
    fn native_same_name_images_are_distinct_and_same_file_aliases_stay_running() {
        let root =
            std::env::temp_dir().join(format!("am-launch-image-{}", crate::store::uuid_v4()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        std::fs::create_dir(&root).unwrap();
        let _cleanup = Cleanup(root.clone());
        for dir in ["first", "second", "alias"] {
            std::fs::create_dir(root.join(dir)).unwrap();
        }
        let first = root.join("first/am-launch-image-fixture.exe");
        let second = root.join("second/am-launch-image-fixture.exe");
        let alias = root.join("alias/am-launch-image-fixture.exe");
        let renamed = root.join("alias/renamed-fixture.exe");
        let source = std::env::current_exe().unwrap();
        std::fs::copy(&source, &first).unwrap();
        std::fs::copy(&source, &second).unwrap();
        std::fs::hard_link(&first, &alias).unwrap();
        std::fs::hard_link(&first, &renamed).unwrap();
        let marker = root.join("ready");
        struct Owned(std::process::Child);
        impl Drop for Owned {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let _child = Owned(
            std::process::Command::new(&first)
                .args([
                    "--exact",
                    "native_launch::runtime::windows::tests::owned_process_helper",
                    "--nocapture",
                ])
                .env("AM_LAUNCH_IMAGE_HELPER", &marker)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(marker.exists(), "Owned image helper did not start");
        assert!(already_running(&first).unwrap());
        assert!(!already_running(&second).unwrap());
        assert!(already_running(&alias).unwrap());
        assert!(already_running(&renamed).unwrap());
    }
}
