use agent_manager_engine::{execute, Context, Request};
use serde_json::json;
use std::path::PathBuf;

fn context() -> Context {
    let root = std::env::temp_dir().join(format!("launch-build-{}", std::process::id()));
    Context {
        home_dir: root.join("home"),
        data_dir: root.join("data"),
    }
}

fn spec(platform: &str, shell: &str) -> serde_json::Value {
    json!({"platform":platform,"shell":shell,"kind":"app","target":"/Applications/Example App.app",
        "args":["O'Brien", "$(touch unexpected)", ""],
        "variants":[{"name":"example-sg","timezone":"Asia/Singapore"},
            {"name":"example-la","timezone":"America/Los_Angeles"}]})
}

#[test]
fn builds_arbitrary_pairs_without_creating_home_or_state() {
    let ctx = context();
    let result = execute(
        &ctx,
        &Request {
            command: "launch:build".into(),
            args: vec!["--spec".into(), spec("macos", "zsh").to_string()],
            run_id: "build".into(),
        },
    );
    assert!(result.ok, "{:?}", result.errors);
    assert_eq!(result.data["entries"].as_array().unwrap().len(), 2);
    assert_eq!(result.data["timezone_verified"], false);
    assert!(!ctx.home_dir.exists());
    assert!(!ctx.data_dir.exists());
}

#[test]
fn rejects_duplicate_names_unknown_fields_and_mismatched_shells() {
    for mut input in [
        spec("windows", "zsh"),
        spec("macos", "zsh"),
        spec("linux", "bash"),
    ] {
        if input["platform"] == "macos" {
            input["variants"][1]["name"] = json!("example-sg");
        }
        if input["platform"] == "linux" {
            input["account"] = json!("not-supported");
        }
        let result = execute(
            &context(),
            &Request {
                command: "launch:build".into(),
                args: vec!["--spec".into(), input.to_string()],
                run_id: "invalid".into(),
            },
        );
        assert!(!result.ok);
    }
}

#[test]
fn windows_shortcut_names_are_case_insensitive_and_not_device_names() {
    for name in ["CON", "com1", "example-SG"] {
        let mut input = spec("windows", "powershell");
        input["variants"][1]["name"] = json!(name);
        let result = execute(
            &context(),
            &Request {
                command: "launch:build".into(),
                args: vec!["--spec".into(), input.to_string()],
                run_id: "windows-name".into(),
            },
        );
        assert!(!result.ok);
    }
}

#[cfg(unix)]
#[test]
fn generated_alias_executes_literal_arguments_under_both_shells() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("launch-literals-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let executable = root.join("example tool");
    std::fs::write(&executable, "#!/bin/sh\nprintf '%s\\n' \"$TZ\" \"$@\"\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    for shell in ["/bin/bash", "/bin/zsh"] {
        if !PathBuf::from(shell).exists() {
            continue;
        }
        let mut input = spec(
            "linux",
            if shell.ends_with("zsh") {
                "zsh"
            } else {
                "bash"
            },
        );
        input["kind"] = json!("command");
        input["target"] = json!(executable);
        let result = execute(
            &context(),
            &Request {
                command: "launch:build".into(),
                args: vec!["--spec".into(), input.to_string()],
                run_id: "literal".into(),
            },
        );
        assert!(result.ok);
        let definition = result.data["entries"][0]["definition"].as_str().unwrap();
        let script =
            format!("shopt -s expand_aliases 2>/dev/null || :\n{definition}\neval example-sg\n");
        let output = std::process::Command::new(shell)
            .arg("-c")
            .arg(script)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "Asia/Singapore\nO'Brien\n$(touch unexpected)\n\n"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
