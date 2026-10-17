//! Actual CLI readback of native uv layouts without running uv or the tool.
use serde_json::Value;
use std::path::Path;
use std::process::Command;

fn status(home: &Path, data: &Path, bin: &Path, tools: &Path) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
        .args(["programs:status", "--json"])
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("AGENT_MANAGER_DATA_DIR", data)
        .env("AGENT_MANAGER_USER_PATH", bin)
        .env("UV_TOOL_DIR", tools)
        .env_remove("AGENT_MANAGER_LOCK_HELD")
        .env_remove("AGENT_MANAGER_LOCK_PID")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], true, "{result}");
    assert!(!data.exists(), "Read-only status created app state");
    result["data"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| {
            !element["installs"].as_array().unwrap().is_empty()
                && element["bins"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|bin| bin == "tool-a")
        })
        .unwrap()
        .clone()
}

#[test]
fn native_cli_uv_version_uses_metadata_and_preserves_hardlinks_and_entries() {
    verify_metadata(false);
}

#[test]
fn native_cli_custom_uv_root_uses_receipt_and_actual_entry() {
    verify_metadata(true);
}

fn verify_metadata(custom: bool) {
    let root = std::env::temp_dir().join(format!("am-uv-status-{}-{custom}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    let home = root.join("home");
    let data = root.join("data");
    let tools = home.join(if custom {
        "custom-extension-tools"
    } else {
        "uv/data/tools"
    });
    let package = tools.join("tool-a");
    let bin = home.join("bin");
    let site = package.join(if cfg!(windows) {
        "Lib/site-packages"
    } else {
        "lib/python3.13/site-packages"
    });
    let metadata = site.join("tool_a-1.2.3.dist-info/METADATA");
    let source = package.join(if cfg!(windows) {
        "Scripts/tool-a.exe"
    } else {
        "bin/tool-a"
    });
    let entry = bin.join(if cfg!(windows) {
        "tool-a.exe"
    } else {
        "tool-a"
    });
    let marker = root.join("unexpected-execution");
    let skill = home.join(".claude/skills/tool-a");
    for directory in [
        metadata.parent().unwrap(),
        source.parent().unwrap(),
        &bin,
        &skill,
    ] {
        std::fs::create_dir_all(directory).unwrap();
    }
    #[cfg(windows)]
    std::fs::write(&source, b"Read-only status must not execute this fixture").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(
            &source,
            format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(windows)]
    std::fs::copy(&source, &entry).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&source, &entry).unwrap();
    std::fs::write(
        package.join("uv-receipt.toml"),
        format!(
            "[tool]\nentrypoints = [{{ name = 'tool-a', install-path = {} }}]\n",
            serde_json::to_string(&entry.to_string_lossy()).unwrap()
        ),
    )
    .unwrap();
    std::fs::write(skill.join("SKILL.md"), "---\nname: tool-a\ndescription: fixture\nmetadata:\n  requires:\n    bins: [tool-a]\n---\nFixture\n").unwrap();
    let contents = "Metadata-Version: 2.4\nName: tool-a\nVersion: 1.2.3\n\n";
    std::fs::write(&metadata, contents).unwrap();
    let cache = root.join("cache-METADATA");
    std::fs::hard_link(&metadata, &cache).unwrap();
    let entry_bytes = std::fs::read(&entry).unwrap();
    let read = || status(&home, &data, &bin, &tools);
    assert_eq!(read()["program"], "pypi:tool-a");
    assert_eq!(read()["version"], "1.2.3");
    std::fs::write(
        &metadata,
        "Metadata-Version: 2.4\nName: tool-a\nVersion: 9.9\n\n",
    )
    .unwrap();
    assert!(
        read()["version"].is_null(),
        "Directory spelling supplied a false version"
    );
    std::fs::write(&metadata, contents).unwrap();
    let duplicate = site.join("tool_a-2.0.dist-info");
    std::fs::create_dir(&duplicate).unwrap();
    assert!(
        read()["version"].is_null(),
        "Unreadable duplicate was ignored"
    );
    std::fs::remove_dir(&duplicate).unwrap();
    std::fs::remove_file(&metadata).unwrap();
    assert!(
        read()["version"].is_null(),
        "Missing METADATA supplied a false version"
    );
    std::fs::hard_link(&cache, &metadata).unwrap();
    assert_eq!(read()["version"], "1.2.3");
    #[cfg(unix)]
    if custom {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let receipt = package.join("uv-receipt.toml");
        let saved = std::fs::read_to_string(&receipt).unwrap();
        let config = home.join(".claude.json");
        std::fs::write(
            &config,
            serde_json::json!({"mcpServers":{"fixture":{"command":entry}}}).to_string(),
        )
        .unwrap();
        let unknown = || {
            let element = read();
            let id = element["program"].as_str().unwrap();
            assert!(id == "bin:tool-a" || id.starts_with("path:"), "{element}");
            assert_eq!(element["owner"], "unknown");
            assert!(!marker.exists());
        };
        for contents in [
            "invalid TOML".to_string(),
            saved.replace("name = 'tool-a'", "name = 'other'"),
            saved.replace("install-path", "other-path"),
            "#".repeat(64 * 1024 + 1),
        ] {
            std::fs::write(&receipt, contents).unwrap();
            unknown();
        }
        std::fs::remove_file(&receipt).unwrap();
        unknown();
        let copy = root.join("receipt-copy");
        std::fs::write(&copy, &saved).unwrap();
        symlink(&copy, &receipt).unwrap();
        unknown();
        std::fs::remove_file(&receipt).unwrap();
        let cpath = std::ffi::CString::new(receipt.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
        unknown();
        std::fs::remove_file(&receipt).unwrap();
        std::fs::write(&receipt, &saved).unwrap();
        for root in [Path::new("relative-tools"), Path::new("")] {
            assert_eq!(status(&home, &data, &bin, root)["program"], "bin:tool-a");
        }
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o644)).unwrap();
        // The absolute MCP path remains inspectable when PATH skips a non-executable source.
        unknown();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_file(&source).unwrap();
        std::fs::create_dir(&source).unwrap();
        unknown();
        std::fs::remove_dir(&source).unwrap();
        let other = package.join("bin/other");
        std::fs::write(&other, &entry_bytes).unwrap();
        std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o755)).unwrap();
        symlink(&other, &source).unwrap();
        unknown();
        std::fs::remove_file(&source).unwrap();
        std::fs::write(&source, &entry_bytes).unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755)).unwrap();
        let invalid_package = tools.join("bad;name");
        std::fs::rename(&package, &invalid_package).unwrap();
        std::fs::remove_file(&entry).unwrap();
        symlink(invalid_package.join("bin/tool-a"), &entry).unwrap();
        unknown();
        std::fs::rename(&invalid_package, &package).unwrap();
        std::fs::remove_file(&entry).unwrap();
        symlink(&source, &entry).unwrap();
        assert_eq!(read()["program"], "pypi:tool-a");
        assert_eq!(read()["version"], "1.2.3");
        std::fs::remove_file(config).unwrap();
    }
    assert_eq!(std::fs::read(&metadata).unwrap(), contents.as_bytes());
    assert_eq!(std::fs::read(&cache).unwrap(), contents.as_bytes());
    assert_eq!(std::fs::read(&entry).unwrap(), entry_bytes);
    assert!(!marker.exists());
    std::fs::remove_dir_all(root).unwrap();
}
