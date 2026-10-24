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
    let value = crate::process::capture_group(&mut command, Duration::from_secs(10), LIMIT)
        .map_err(|_| anyhow::anyhow!("Running Claude processes could not be checked."))?;
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
            let arguments = String::from_utf8_lossy(&arguments).replace('\0', " ");
            if is_claude(comm.trim(), &arguments) || is_claude(&image, &arguments) {
                pids.push(pid);
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
        command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; $s=(Get-Process -Id $PID).SessionId; ConvertTo-Json -Compress -InputObject @(Get-CimInstance Win32_Process -Filter \"SessionId = $s\" | Select-Object ProcessId,Name,CommandLine)"]);
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
            if matches!(image.to_lowercase().as_str(), "node.exe" | "bun.exe")
                && row["CommandLine"].as_str().is_none()
            {
                bail!("An editor runtime could not be inspected; cleanup is unavailable.");
            }
            if is_claude(image, row["CommandLine"].as_str().unwrap_or("")) {
                pids.push(
                    row["ProcessId"]
                        .as_u64()
                        .ok_or_else(|| anyhow::anyhow!("Process identity is unavailable."))?
                        as u32,
                );
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

    #[cfg(target_os = "macos")]
    #[test]
    fn process_paths_with_spaces_are_not_split_into_a_different_executable() {
        assert_eq!(
            process_line(" 123 /YOUR_HOME/Claude Code/claude"),
            Some((123, "/YOUR_HOME/Claude Code/claude"))
        );
    }
}
