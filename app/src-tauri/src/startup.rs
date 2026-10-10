//! Desktop launch location. Reading it never changes configuration or OS state.

pub(crate) fn location_warning() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let executable = std::env::current_exe().ok()?;
        warning_for(&executable, readonly_mount(&executable)).map(str::to_string)
    }
    #[cfg(not(target_os = "macos"))]
    None
}

#[cfg(target_os = "macos")]
fn readonly_mount(path: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut mount: libc::statfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statfs(path.as_ptr(), &mut mount) }) == 0
        && mount.f_flags & libc::MNT_RDONLY as u32 != 0
}

#[cfg(any(target_os = "macos", test))]
fn warning_for(executable: &std::path::Path, readonly: bool) -> Option<&'static str> {
    let translocated = executable
        .components()
        .any(|part| part.as_os_str() == "AppTranslocation");
    if readonly || translocated {
        Some("请先退出应用，将 Agent Manager 拖到“应用程序”文件夹后再打开。当前位置无法可靠保存更新或开机启动设置。")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[cfg(target_os = "macos")]
    #[test]
    fn actual_system_and_development_mounts_are_distinguished() {
        assert!(readonly_mount(Path::new("/System/Library")));
        assert!(!readonly_mount(&std::env::current_exe().unwrap()));
        assert!(location_warning().is_none());
    }

    #[test]
    fn only_readonly_mounts_and_translocated_paths_warn() {
        assert!(warning_for(
            Path::new("/Volumes/Installer/Agent Manager.app/Contents/MacOS/app"),
            true
        )
        .is_some());
        assert!(warning_for(Path::new("/private/var/folders/example/AppTranslocation/id/d/Agent Manager.app/Contents/MacOS/app"), false).is_some());
        assert!(warning_for(
            Path::new("/Volumes/Work/Agent Manager.app/Contents/MacOS/app"),
            false
        )
        .is_none());
        assert!(warning_for(
            Path::new("/Applications/Agent Manager.app/Contents/MacOS/app"),
            false
        )
        .is_none());
    }
}
