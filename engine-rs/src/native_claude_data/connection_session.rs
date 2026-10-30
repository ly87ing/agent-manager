//! Current graphical login environment only; caller variables are never changed.
use anyhow::{bail, Result};

#[cfg(target_os = "macos")]
fn command(args: &[&str]) -> Result<Vec<u8>> {
    use std::{process::Stdio, time::Duration};
    let mut command = crate::process::launch_command("/bin/launchctl");
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = crate::process::capture_group(&mut command, Duration::from_secs(3), 16384)?;
    if !output.status.success() || output.stdout.len() >= 16384 {
        bail!("User login environment is unavailable.");
    }
    Ok(output.stdout)
}

#[cfg(target_os = "macos")]
pub(super) fn scope() -> Result<String> {
    #[repr(C)]
    struct Token {
        words: [u32; 8],
    }
    #[link(name = "bsm")]
    unsafe extern "C" {
        fn audit_token_to_asid(token: Token) -> libc::pid_t;
    }
    let uid = unsafe { libc::geteuid() };
    if command(&["managername"])? != b"Aqua\n"
        || String::from_utf8(command(&["manageruid"])?)?.trim() != uid.to_string()
    {
        bail!("A current-user graphical login session is required.");
    }
    let mut token = Token { words: [0; 8] };
    let mut count = 8;
    #[allow(deprecated)]
    if unsafe {
        libc::task_info(
            libc::mach_task_self(),
            15,
            token.words.as_mut_ptr().cast(),
            &mut count,
        )
    } != 0
        || count != 8
    {
        bail!("Login identity is unavailable.");
    }
    let asid = unsafe { audit_token_to_asid(token) };
    if asid <= 0 {
        bail!("Login identity is unavailable.");
    }
    // Do not print the session, whose environment can contain unrelated credentials.
    use std::{process::Stdio, time::Duration};
    let login = format!("login/{asid}");
    let mut check = crate::process::launch_command("/bin/launchctl");
    check
        .args(["print", &login])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !crate::process::capture_group(&mut check, Duration::from_secs(3), 1024)?
        .status
        .success()
    {
        bail!("Login identity is unavailable.");
    }
    Ok(format!("gui/{uid} · {login} · launchd Aqua"))
}

#[cfg(not(target_os = "macos"))]
pub(super) fn scope() -> Result<String> {
    bail!("User environment cleanup is currently available on macOS only.");
}

pub(super) fn read(key: &str) -> Result<Option<Vec<u8>>> {
    if !super::connection_inspect::is_key(key) {
        bail!("Unsupported connection variable.");
    }
    #[cfg(target_os = "macos")]
    {
        let mut bytes = command(&["getenv", key])?;
        if bytes.last() == Some(&b'\n') {
            bytes.pop();
        }
        return Ok((!bytes.is_empty()).then_some(bytes));
    }
    #[cfg(not(target_os = "macos"))]
    bail!("User environment is unavailable.");
}

fn remove_guarded(
    expected_scope: &str,
    expected: &[u8],
    scope: impl FnOnce() -> Result<String>,
    read: impl Fn() -> Result<Option<Vec<u8>>>,
    unset: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if scope()? != expected_scope || read()?.as_deref() != Some(expected) {
        bail!("The selected user environment changed; request a new preview.");
    }
    // launchctl has no compare-and-swap API; other writers must remain stopped.
    unset()?;
    if read()?.is_some() {
        bail!("User environment removal did not verify.");
    }
    Ok(())
}

pub(super) fn remove(key: &str, expected_scope: &str, expected: &[u8]) -> Result<()> {
    if !super::connection_inspect::shared_key(key) {
        bail!("General shared proxy variables are inspection-only.");
    }
    #[cfg(target_os = "macos")]
    {
        return remove_guarded(
            expected_scope,
            expected,
            scope,
            || read(key),
            || command(&["unsetenv", key]).map(|_| ()),
        );
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (expected_scope, expected);
        bail!("User environment cleanup is unavailable.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn change_during_scope_verification_never_unsets_the_new_value() {
        let value = std::cell::RefCell::new(Some(b"OLD".to_vec()));
        let unset = std::cell::Cell::new(false);
        assert!(remove_guarded(
            "scope",
            b"OLD",
            || {
                *value.borrow_mut() = Some(b"NEW".to_vec());
                Ok("scope".into())
            },
            || Ok(value.borrow().clone()),
            || {
                unset.set(true);
                *value.borrow_mut() = None;
                Ok(())
            }
        )
        .is_err());
        assert!(!unset.get());
        assert_eq!(value.into_inner(), Some(b"NEW".to_vec()));
    }
}
