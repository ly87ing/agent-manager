//! Native Windows fixture contracts; cross-compilation does not execute these checks.
#![cfg(windows)]

mod common;
use agent_manager_engine::{execute, Context, Request};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

struct Root(PathBuf);
impl Root {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "am-windows-fixture-{}-{tag}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        common::private_fixture_root(&path);
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn run(ctx: &Context, command: &str, args: &[&str]) -> agent_manager_engine::Envelope {
    execute(
        ctx,
        &Request {
            command: command.into(),
            args: args.iter().map(|s| (*s).into()).collect(),
            run_id: "windows-fixture-test".into(),
        },
    )
}

#[test]
fn private_source_and_atomic_replacement_inherit_restricted_acl_and_broad_acl_is_rejected() {
    let root = Root::new("private-source");
    let ctx = Context {
        home_dir: root.0.join("home"),
        data_dir: root.0.join("data"),
    };
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    std::fs::write(
        ctx.home_dir.join(".claude.json"),
        json!({"mcpServers": {"fixture": {
            "command": "YOUR_FAKE_MCP_COMMAND",
            "env": {"API_KEY": "${YOUR_FIXTURE_KEY}"}
        }}})
        .to_string(),
    )
    .unwrap();
    let source = ctx.home_dir.join("private/account.env");
    let selected = run(
        &ctx,
        "secrets:source",
        &["--path", source.to_str().unwrap(), "--create"],
    );
    assert!(selected.ok, "{:?}", selected.errors);
    for value in ["YOUR_FAKE_FIRST_VALUE", "YOUR_FAKE_SECOND_VALUE"] {
        let changed = run(
            &ctx,
            "secrets:set",
            &["--var", "YOUR_FIXTURE_KEY", "--value", value],
        );
        assert!(changed.ok, "{:?}", changed.errors);
        assert!(std::fs::read_to_string(&source).unwrap().contains(value));
    }
    let before = std::fs::read(&source).unwrap();
    grant_everyone_to_fixture_file(&source);
    let rejected = run(
        &ctx,
        "secrets:set",
        &[
            "--var",
            "YOUR_FIXTURE_KEY",
            "--value",
            "YOUR_REJECTED_VALUE",
        ],
    );
    assert!(!rejected.ok);
    assert!(
        rejected
            .errors
            .iter()
            .any(|e| e.message.contains("ACL") && e.message.contains("其他用户")),
        "{:?}",
        rejected.errors
    );
    assert_eq!(std::fs::read(&source).unwrap(), before);
}

// This deliberately broad ACL belongs only to a disposable fake source, never a user file.
fn grant_everyone_to_fixture_file(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{Security::*, Storage::FileSystem::FILE_ALL_ACCESS};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let mut sid = [0u32; 17];
        let mut size = (sid.len() * 4) as u32;
        assert_ne!(
            CreateWellKnownSid(
                WinWorldSid,
                std::ptr::null_mut(),
                sid.as_mut_ptr().cast(),
                &mut size
            ),
            0
        );
        let mut acl_storage = [0u32; 32];
        let acl = acl_storage.as_mut_ptr().cast();
        assert_ne!(
            InitializeAcl(acl, (acl_storage.len() * 4) as u32, ACL_REVISION),
            0
        );
        assert_ne!(
            AddAccessAllowedAceEx(
                acl,
                ACL_REVISION,
                0,
                FILE_ALL_ACCESS,
                sid.as_mut_ptr().cast()
            ),
            0
        );
        let mut descriptor: SECURITY_DESCRIPTOR = std::mem::zeroed();
        let ptr = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
        assert_ne!(InitializeSecurityDescriptor(ptr, 1), 0);
        assert_ne!(SetSecurityDescriptorDacl(ptr, 1, acl, 0), 0);
        assert_ne!(
            SetFileSecurityW(
                wide.as_ptr(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr
            ),
            0
        );
    }
}

#[test]
fn native_fake_clis_preserve_versions_probe_arguments_and_actual_launch_input() {
    let root = Root::new("native-cli");
    let home = root.0.join("home");
    common::install_fake_account_clis(&home);
    let bin = home.join(".local/bin");
    for (name, version) in [
        ("claude.exe", "2.1.285 (Claude Code)"),
        ("codex.exe", "codex-cli 0.156.1"),
    ] {
        let output = std::process::Command::new(bin.join(name))
            .arg("--version")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), version);
    }
    let directory = home.join(".claude-test");
    std::fs::create_dir_all(&directory).unwrap();
    let settings = json!({"apiKeyHelper":"", "env":{"ANTHROPIC_AUTH_TOKEN":""}}).to_string();
    let output = std::process::Command::new(bin.join("claude.exe"))
        .env_clear()
        .env("HOME", &home)
        .env("CLAUDE_CONFIG_DIR", &directory)
        .args(["--settings", &settings, "auth", "status", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["loggedIn"],
        false
    );
    assert_eq!(
        std::fs::read_to_string(home.join(".probe-args"))
            .unwrap()
            .trim(),
        format!("{settings}|unset|unset")
    );
    std::fs::write(directory.join(".fake-logged-in"), "").unwrap();
    let logged_in = std::process::Command::new(bin.join("claude.exe"))
        .env_clear()
        .env("HOME", &home)
        .env("CLAUDE_CONFIG_DIR", &directory)
        .args(["auth", "status", "--json"])
        .output()
        .unwrap();
    assert!(logged_in.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&logged_in.stdout).unwrap()["loggedIn"],
        true
    );
    let args = [
        "--model",
        "fixture-model",
        " extra arg ",
        "quote\"and\\slash",
    ];
    let launched = std::process::Command::new(bin.join("claude.exe"))
        .env_clear()
        .env("HOME", &home)
        .env("CLAUDE_CONFIG_DIR", &directory)
        .env("TZ", "UTC")
        .args(args)
        .status()
        .unwrap();
    assert!(launched.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(home.join(".fake-cli-argv")).unwrap())
            .unwrap(),
        json!(args)
    );
    let env: Value =
        serde_json::from_slice(&std::fs::read(home.join(".fake-cli-env")).unwrap()).unwrap();
    assert_eq!(env["TZ"], "UTC");
    assert_eq!(env["CLAUDE_CONFIG_DIR"], directory.to_str().unwrap());
    assert_eq!(env["ANTHROPIC_AUTH_TOKEN"], "unset");
}
