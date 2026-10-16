//! Bounded OS-tool reads; no shell expansion, profiles or caller TZ override.
#[cfg(any(target_os = "linux", windows))]
use std::{process::Stdio, time::Duration};

#[cfg(target_os = "linux")]
pub(super) fn read(program: &str, args: &[&str]) -> Option<String> {
    bounded(program, args, Duration::from_secs(8))
}

#[cfg(any(target_os = "linux", windows))]
pub(super) fn bounded(program: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut command = crate::process::launch_command(program);
    #[cfg(windows)]
    if std::path::Path::new(program)
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("powershell.exe"))
    {
        // NoProfile does not disable module-cache writes or user module discovery.
        command.env("PSModuleAnalysisCachePath", "NUL");
        command.env(
            "PSModulePath",
            std::path::Path::new(program).parent()?.join("Modules"),
        );
    }
    command
        .args(args)
        .env("LC_ALL", "C")
        .env_remove("TZ")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = crate::process::capture_group(&mut command, timeout, 64 * 1024).ok()?;
    if !output.status.success() || output.stdout.len() >= 64 * 1024 {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|text| text.trim_start_matches('\u{feff}').trim().to_string())
}

#[cfg(any(target_os = "linux", windows))]
pub(super) fn process_environment() -> std::collections::BTreeMap<String, String> {
    ["TZ", "LANG", "LC_ALL"]
        .iter()
        .chain(super::LOCALE_CATEGORIES.iter())
        .filter_map(|key| {
            std::env::var_os(key).map(|value| {
                (
                    (*key).into(),
                    value.to_string_lossy().chars().take(512).collect(),
                )
            })
        })
        .collect()
}
