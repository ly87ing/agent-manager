use agent_manager_engine::{execute, Context, Request};
use serde_json::json;

fn context() -> Context {
    let root = std::env::temp_dir().join(format!(
        "environment-repair-{}",
        agent_manager_engine::store::uuid_v4()
    ));
    Context {
        home_dir: root.join("home"),
        data_dir: root.join("data"),
    }
}

fn request(command: &str, args: Vec<String>) -> Request {
    Request {
        command: command.into(),
        args,
        run_id: "repair-contract".into(),
    }
}

#[test]
fn repair_preview_is_read_only_and_binds_the_selected_targets() {
    let ctx = context();
    let spec = json!({"timezone":"America/Los_Angeles", "locale":"en_US.UTF-8", "ntp_server":"time.invalid", "automatic_time":true});
    let result = execute(
        &ctx,
        &request(
            "environment:repair-preview",
            vec!["--spec".into(), spec.to_string()],
        ),
    );
    #[cfg(target_os = "macos")]
    {
        assert!(result.ok, "{:?}", result.errors);
        assert_eq!(result.data["base"].as_str().unwrap().len(), 64);
        let items = result.data["items"].as_array().unwrap();
        assert!(items.iter().any(|item| item["id"] == "time.timezone"));
        assert!(items
            .iter()
            .any(|item| item["id"] == "session.LC_CTYPE" && item["expected"] == "en_US.UTF-8"));
        assert!(items
            .iter()
            .any(|item| item["id"] == "ntp.server" && item["expected"] == "time.invalid"));
        assert!(result.data["session_scope"]
            .as_str()
            .unwrap()
            .contains("launchd"));
    }
    #[cfg(windows)]
    {
        assert!(result.ok, "{:?}", result.errors);
        assert_eq!(result.data["platform"], "windows");
        assert!(result.data["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["id"].as_str().unwrap().starts_with("session."))
            .all(|item| item["status"] == "unavailable"));
    }
    #[cfg(target_os = "linux")]
    {
        assert!(result.ok, "{:?}", result.errors);
        assert_eq!(result.data["platform"], "linux");
        assert!(result.data["session_scope"]
            .as_str()
            .unwrap()
            .contains("systemd user manager"));
        assert!(result.data["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "time.timezone"
                && item["expected"] == "America/Los_Angeles"
                && item["scope"] == "system"));
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    assert!(!result.ok);
    assert!(result.mode.offline);
    assert!(!ctx.home_dir.exists());
    assert!(!ctx.data_dir.exists());
}

#[test]
fn repair_rejects_missing_confirmation_base_and_unvalidated_targets_without_state() {
    let ctx = context();
    for args in [
        vec![],
        vec!["--spec", "{}", "--confirm"],
        vec![
            "--spec",
            "{\"region\":\"US\"}",
            "--base",
            "bad",
            "--confirm",
        ],
        vec![
            "--spec",
            "{\"timezone\":\"../../etc\"}",
            "--base",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--confirm",
        ],
        vec![
            "--spec",
            "{\"locale\":\"en_US.UTF-8; id\"}",
            "--base",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--confirm",
        ],
    ] {
        let result = execute(
            &ctx,
            &request(
                "environment:repair",
                args.into_iter().map(str::to_string).collect(),
            ),
        );
        assert!(!result.ok);
    }
    let result = execute(
        &ctx,
        &request(
            "environment:repair-preview",
            vec![
                "--spec".into(),
                "{\"region\":\"US\"}".into(),
                "--confirm".into(),
            ],
        ),
    );
    assert!(!result.ok);
    assert!(!ctx.home_dir.exists());
    assert!(!ctx.data_dir.exists());
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn confirmed_noop_uses_real_settings_without_writing_preferences_or_database() {
    let ctx = context();
    let observed = execute(&ctx, &request("environment:check", vec![]));
    assert!(observed.ok);
    let region = observed.data["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "language.region")
        .unwrap()["observed"]
        .as_str()
        .unwrap();
    let spec = json!({"region":region}).to_string();
    let preview = execute(
        &ctx,
        &request(
            "environment:repair-preview",
            vec!["--spec".into(), spec.clone()],
        ),
    );
    assert!(preview.ok, "{:?}", preview.errors);
    assert!(
        preview.data["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["status"] == "unchanged"),
        "The real acceptance must remain a no-op"
    );
    let result = execute(
        &ctx,
        &request(
            "environment:repair",
            vec![
                "--spec".into(),
                spec,
                "--base".into(),
                preview.data["base"].as_str().unwrap().into(),
                "--confirm".into(),
            ],
        ),
    );
    assert!(result.ok, "{:?}", result.errors);
    assert_eq!(result.data["status"], "not_needed");
    assert_eq!(result.data["report"]["summary"]["status"], "match");
    assert!(result.data["report"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["id"] == "language.region" && row["observed"] == region));
    assert!(!ctx.home_dir.exists());
    assert!(!ctx.data_dir.exists());
}
