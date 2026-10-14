use agent_manager_engine::{execute, Context, Request};

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
    #[cfg(target_os = "macos")]
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
            assert_eq!(sync["state"], "unknown");
            assert!(sync["observed"].is_null());
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
    #[cfg(not(target_os = "macos"))]
    assert!(!result.ok);
    assert!(result.mode.offline);
    assert!(!ctx.home_dir.exists());
    assert!(!ctx.data_dir.exists());
}
