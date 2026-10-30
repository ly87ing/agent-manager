//! Only executable names are observed. No third-party process is terminated.
pub(super) fn cc_switch() -> &'static str {
    match executable_names() {
        Ok(names)
            if names.iter().any(|name| {
                matches!(
                    name.replace(' ', "").to_ascii_lowercase().as_str(),
                    "cc-switch" | "ccswitch" | "cc-switch.exe" | "ccswitch.exe"
                )
            }) =>
        {
            "running"
        }
        Ok(_) => "not_running",
        Err(_) => "unknown",
    }
}

#[cfg(unix)]
fn executable_names() -> anyhow::Result<Vec<String>> {
    use std::{process::Stdio, time::Duration};
    let uid = unsafe { libc::geteuid() }.to_string();
    let mut command = crate::process::launch_command("/bin/ps");
    command
        .args(["-U", &uid, "-o", "comm="])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output =
        crate::process::capture_group(&mut command, Duration::from_secs(10), 4 * 1024 * 1024)?;
    anyhow::ensure!(
        output.status.success() && output.stdout.len() < 4 * 1024 * 1024,
        "Process inspection unavailable."
    );
    Ok(String::from_utf8(output.stdout)?
        .lines()
        .map(|line| line.trim().rsplit('/').next().unwrap_or("").to_string())
        .collect())
}

#[cfg(windows)]
fn executable_names() -> anyhow::Result<Vec<String>> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::*,
    };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    anyhow::ensure!(
        snapshot != INVALID_HANDLE_VALUE,
        "Process inspection unavailable."
    );
    let result = (|| {
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of_val(&entry) as u32;
        let mut names = Vec::new();
        let mut found = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
        while found {
            let end = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
            if name.to_ascii_lowercase().contains("switch")
                && super::process_stop::current_user(entry.th32ProcessID)?
            {
                names.push(name);
            }
            found = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
        }
        Ok(names)
    })();
    unsafe {
        CloseHandle(snapshot);
    }
    result
}
