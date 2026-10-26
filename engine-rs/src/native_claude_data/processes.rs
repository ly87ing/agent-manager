//! Only process metadata is inspected; command lines are never returned or logged.
use anyhow::{bail, Result};

fn is_claude(image: &str, arguments: &str) -> bool {
    let image = image.replace('\\', "/").to_lowercase();
    let name = image.rsplit('/').next().unwrap_or("");
    if matches!(
        name,
        "claude" | "claude.exe" | "claude-code" | "claude-code.exe"
    ) {
        return true;
    }
    if image.contains("/claude.app/") {
        return true;
    }
    matches!(name, "node" | "node.exe" | "bun" | "bun.exe")
        && arguments
            .replace('\\', "/")
            .contains("/@anthropic-ai/claude-code/")
}

fn is_runtime(image: &str) -> bool {
    matches!(
        image
            .replace('\\', "/")
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_lowercase()
            .as_str(),
        "node" | "node.exe" | "bun" | "bun.exe"
    )
}

fn verify_runtime(image: &str, arguments: &[String]) -> Result<()> {
    if is_runtime(image)
        && !arguments.get(1).is_some_and(|script| {
            let script = script.replace('\\', "/");
            script.contains("/@anthropic-ai/claude-code/") && script.ends_with("/cli.js")
        })
    {
        bail!("A possible Claude runtime has ambiguous launch arguments; close it manually and request a new preview.");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn runtime_arguments(pid: u32) -> Result<Vec<String>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as i32];
    let mut bytes = vec![0u8; 64 * 1024];
    let mut length = bytes.len();
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            bytes.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || length < 4
        || length >= bytes.len()
    {
        bail!("A Claude runtime's launch arguments could not be verified.");
    }
    bytes.truncate(length);
    let count = i32::from_ne_bytes(bytes[..4].try_into().unwrap());
    if !(2..=1024).contains(&count) {
        bail!("A Claude runtime's argument count is unavailable.");
    }
    let executable_end = bytes[4..]
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| anyhow::anyhow!("A Claude runtime's executable is unavailable."))?
        + 4;
    let start = (executable_end..bytes.len())
        .find(|i| bytes[*i] != 0)
        .ok_or_else(|| anyhow::anyhow!("A Claude runtime's arguments are unavailable."))?;
    bytes[start..]
        .split(|b| *b == 0)
        .take(count as usize)
        .map(|arg| String::from_utf8(arg.to_vec()).map_err(Into::into))
        .collect()
}

#[cfg(target_os = "macos")]
fn process_line(line: &str) -> Option<(u32, &str)> {
    let (pid, rest) = line.trim().split_once(char::is_whitespace)?;
    Some((pid.parse().ok()?, rest.trim()))
}

#[cfg(any(target_os = "macos", windows))]
fn output(mut command: std::process::Command) -> Result<String> {
    use std::{process::Stdio, time::Duration};
    const LIMIT: usize = 4 * 1024 * 1024;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let timeout = Duration::from_secs(if cfg!(windows) { 30 } else { 10 });
    let value = crate::process::capture_group(&mut command, timeout, LIMIT).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            "Running Claude processes could not be checked.",
        )
    })?;
    if !value.status.success() || value.stdout.len() >= LIMIT {
        bail!("Running Claude processes could not be checked.");
    }
    String::from_utf8(value.stdout)
        .map_err(|_| anyhow::anyhow!("Running Claude processes could not be checked."))
}

pub(super) fn active() -> Result<Vec<u32>> {
    let mut pids = Vec::new();
    #[cfg(target_os = "macos")]
    {
        let mut command = crate::process::launch_command("/bin/ps");
        let user = unsafe { libc::geteuid() }.to_string();
        command.args(["-U", &user, "-ww", "-o", "pid=,comm="]);
        let images = output(command)?;
        let mut command = crate::process::launch_command("/bin/ps");
        command.args(["-U", &user, "-ww", "-o", "pid=,args="]);
        let arguments = output(command)?;
        let arguments: std::collections::BTreeMap<_, _> =
            arguments.lines().filter_map(process_line).collect();
        for (pid, image) in images.lines().filter_map(process_line) {
            if is_claude(image, arguments.get(&pid).copied().unwrap_or("")) {
                if is_runtime(image) {
                    verify_runtime(image, &runtime_arguments(pid)?)?;
                }
                pids.push(pid);
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::io::Read;
        use std::os::unix::fs::MetadataExt;
        for entry in std::fs::read_dir("/proc")? {
            let entry = entry?;
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|v| v.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.uid() != unsafe { libc::geteuid() } {
                continue;
            }
            let stat = match std::fs::read_to_string(entry.path().join("stat")) {
                Ok(value) => value,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => bail!("A current-user process state could not be checked."),
            };
            if stat
                .rsplit_once(')')
                .is_some_and(|(_, rest)| rest.trim_start().starts_with("Z "))
            {
                continue;
            }
            let comm = match std::fs::read_to_string(entry.path().join("comm")) {
                Ok(value) => value,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => bail!("A current-user process could not be checked."),
            };
            let mut arguments = Vec::new();
            match std::fs::File::open(entry.path().join("cmdline")) {
                Ok(file) => {
                    file.take(64 * 1024 + 1).read_to_end(&mut arguments)?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => bail!("A current-user process could not be checked."),
            }
            if arguments.len() > 64 * 1024 {
                bail!("A process command line exceeds the inspection limit.");
            }
            // argv[0] is NUL-delimited, so spaces in an executable path stay intact.
            // Runtime thread names can replace comm without replacing this argument.
            let image =
                String::from_utf8_lossy(arguments.split(|b| *b == 0).next().unwrap_or_default());
            let argv: Vec<_> = arguments
                .split(|b| *b == 0)
                .map(|arg| String::from_utf8_lossy(arg).into_owned())
                .collect();
            let arguments = String::from_utf8_lossy(&arguments).replace('\0', " ");
            if is_claude(comm.trim(), &arguments) || is_claude(&image, &arguments) {
                let executable = std::fs::read_link(entry.path().join("exe"))?;
                if is_claude(&executable.to_string_lossy(), &arguments) {
                    verify_runtime(&executable.to_string_lossy(), &argv)?;
                    pids.push(pid);
                } else {
                    bail!("A Claude process name does not match its executable; cleanup is unavailable.");
                }
            }
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
        let mut path = vec![0u16; 32768];
        let length = unsafe { GetSystemDirectoryW(path.as_mut_ptr(), path.len() as u32) } as usize;
        if length == 0 || length >= path.len() {
            bail!("Windows system tools could not be located.");
        }
        let root = std::path::PathBuf::from(String::from_utf16(&path[..length])?);
        let executable = root.join("WindowsPowerShell/v1.0/powershell.exe");
        let mut command = crate::process::launch_command(&executable);
        command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; ConvertTo-Json -Compress -InputObject @(Get-CimInstance Win32_Process | Select-Object ProcessId,Name,CommandLine)"]);
        command
            .env("PSModuleAnalysisCachePath", "NUL")
            .env("PSModulePath", executable.parent().unwrap().join("Modules"));
        let text = output(command)?;
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(text.trim_start_matches('\u{feff}'))
                .map_err(|_| anyhow::anyhow!("Running Claude processes could not be checked."))?;
        for row in rows {
            let image = row["Name"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Process metadata is unavailable."))?;
            if !is_claude(image, "") && !is_runtime(image) {
                continue;
            }
            let pid = row["ProcessId"]
                .as_u64()
                .and_then(|pid| u32::try_from(pid).ok())
                .filter(|pid| *pid != 0)
                .ok_or_else(|| anyhow::anyhow!("Process identity is unavailable."))?;
            if !super::process_stop::current_user(pid)? {
                continue;
            }
            if matches!(image.to_lowercase().as_str(), "node.exe" | "bun.exe")
                && row["CommandLine"].as_str().is_none()
            {
                bail!("An editor runtime could not be inspected; cleanup is unavailable.");
            }
            if is_claude(image, row["CommandLine"].as_str().unwrap_or("")) {
                if is_runtime(image) {
                    use windows_sys::Win32::{
                        Foundation::LocalFree, UI::Shell::CommandLineToArgvW,
                    };
                    let wide: Vec<_> = row["CommandLine"]
                        .as_str()
                        .unwrap()
                        .encode_utf16()
                        .chain(Some(0))
                        .collect();
                    let mut count = 0;
                    let pointer = unsafe { CommandLineToArgvW(wide.as_ptr(), &mut count) };
                    if pointer.is_null() {
                        bail!("A Claude runtime's launch arguments are unavailable.");
                    }
                    let mut arguments = Vec::new();
                    if (2..=1024).contains(&count) {
                        for index in 0..count as usize {
                            let arg = unsafe { *pointer.add(index) };
                            let mut length = 0;
                            while unsafe { *arg.add(length) } != 0 {
                                length += 1;
                            }
                            arguments.push(String::from_utf16_lossy(unsafe {
                                std::slice::from_raw_parts(arg, length)
                            }));
                        }
                    }
                    unsafe {
                        LocalFree(pointer.cast());
                    }
                    verify_runtime(image, &arguments)?;
                }
                pids.push(pid);
            }
        }
    }
    pids.sort_unstable();
    pids.dedup();
    Ok(pids)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_and_node_cli_are_blocked_without_mistaking_manager_or_other_assistants() {
        assert!(is_claude("/usr/local/bin/claude", ""));
        assert!(is_claude("C:\\tools\\claude.exe", ""));
        assert!(is_claude(
            "node",
            "/YOUR_HOME/node_modules/@anthropic-ai/claude-code/cli.js"
        ));
        assert!(!is_claude("agent-manager-engine", "claude-data:preview"));
        assert!(!is_claude("codex", "--context claude-code"));
        assert!(!is_claude("node", "/YOUR_PROJECT/unrelated.js"));
        assert!(is_claude("/YOUR_HOME/Applications/Claude Code/claude", ""));
    }

    #[test]
    fn runtime_arguments_must_launch_the_cli_not_merely_mention_it() {
        let cli = "/YOUR HOME/node_modules/@anthropic-ai/claude-code/cli.js";
        assert!(verify_runtime("node", &["node".into(), cli.into()]).is_ok());
        assert!(verify_runtime("node", &["node".into(), "other.js".into(), cli.into()]).is_err());
        assert!(verify_runtime("node", &["node".into(), "-e".into(), cli.into()]).is_err());
        assert!(verify_runtime("codex", &[cli.into()]).is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn process_paths_with_spaces_are_not_split_into_a_different_executable() {
        assert_eq!(
            process_line(" 123 /YOUR_HOME/Claude Code/claude"),
            Some((123, "/YOUR_HOME/Claude Code/claude"))
        );
    }
}
