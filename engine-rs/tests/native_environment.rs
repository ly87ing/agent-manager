use agent_manager_engine::{execute, Context, Request};

#[cfg(windows)]
#[test]
fn windows_inspection_does_not_load_user_modules_or_write_module_cache() {
    let root = std::env::temp_dir().join(format!("am-environment-ps-{}", std::process::id()));
    let module = root.join("modules/International");
    std::fs::create_dir_all(&module).unwrap();
    let marker = root.join("unexpected-module");
    std::fs::write(
        module.join("International.psm1"),
        format!(
            "[IO.File]::WriteAllText('{}','loaded')",
            marker.to_string_lossy().replace('\'', "''")
        ),
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
        .args(["environment:check", "--json"])
        .env("HOME", root.join("home"))
        .env("AGENT_MANAGER_DATA_DIR", root.join("data"))
        .env("LOCALAPPDATA", root.join("local"))
        .env("PSModulePath", root.join("modules"))
        .env("PSModuleAnalysisCachePath", root.join("unexpected-cache"))
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], true, "{value}");
    assert!(!marker.exists());
    assert!(!root.join("unexpected-cache").exists());
    assert!(!root.join("local").exists());
    assert!(!root.join("home").exists());
    assert!(!root.join("data").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn environment_check_does_not_create_home_or_data() {
    let root = std::env::temp_dir().join(format!("environment-check-{}", std::process::id()));
    let ctx = Context {
        home_dir: root.join("missing-home"),
        data_dir: root.join("missing-data"),
    };
    let result = execute(
        &ctx,
        &Request {
            command: "environment:check".into(),
            args: vec![],
            run_id: "read-only".into(),
        },
    );
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    {
        assert!(result.ok, "{:?}", result.errors);
        assert!(result.data["checks"].as_array().unwrap().len() >= 20);
        assert!(result.data["time_sample"].is_null());
        let sync = result.data["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == "ntp.synchronized")
            .unwrap();
        if result.data["time_sync"].is_null() {
            #[cfg(not(target_os = "linux"))]
            {
                assert_eq!(sync["state"], "unknown");
                assert!(sync["observed"].is_null());
            }
            #[cfg(target_os = "linux")]
            match sync["observed"].as_str() {
                Some("synchronized" | "unsynchronized") => assert_eq!(sync["state"], "observed"),
                None => assert_eq!(sync["state"], "unknown"),
                other => panic!("Invalid Linux kernel synchronization observation: {other:?}"),
            }
        } else {
            let synchronized = result.data["time_sync"]["synchronized"].as_bool().unwrap();
            assert_eq!(
                sync["observed"],
                if synchronized {
                    "synchronized"
                } else {
                    "unsynchronized"
                }
            );
            assert!(result.data["time_sync"]["max_error_ms"].as_f64().unwrap() >= 0.0);
        }
        assert_eq!(result.data["summary"]["status"], "incomplete");
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    assert!(!result.ok);
    assert!(result.mode.offline);
    assert!(!ctx.home_dir.exists());
    assert!(!ctx.data_dir.exists());
}

#[test]
fn remediation_rejects_unconfirmed_reads_and_arbitrary_settings_before_any_effect() {
    let root = std::env::temp_dir().join(format!("environment-actions-{}", std::process::id()));
    let ctx = Context {
        home_dir: root.join("missing-home"),
        data_dir: root.join("missing-data"),
    };
    for (command, args) in [
        ("environment:check-browser", vec![]),
        (
            "environment:check-browser",
            vec!["--confirm", "--spec", "{\"timezone\":\"../../etc\"}"],
        ),
        (
            "environment:check-browser",
            vec!["--confirm", "--probe-time"],
        ),
        ("environment:check-browser", vec!["--confirm", "--confirm"]),
        ("environment:check-time-settings", vec![]),
        (
            "environment:check-time-settings",
            vec!["--confirm", "--spec", "{\"timezone\":\"../../etc\"}"],
        ),
        (
            "environment:open-settings",
            vec!["--pane", "https://example.org"],
        ),
        (
            "environment:open-settings",
            vec!["--pane", "date-time", "--spec", "{}"],
        ),
    ] {
        let result = execute(
            &ctx,
            &Request {
                command: command.into(),
                args: args.into_iter().map(str::to_string).collect(),
                run_id: "invalid-action".into(),
            },
        );
        assert!(!result.ok, "{:?}", result.data);
    }
    assert!(!ctx.home_dir.exists());
    assert!(!ctx.data_dir.exists());
}
