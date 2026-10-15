use agent_manager_engine::{execute, store, Context, Request};
use serde_json::{json, Value};

fn context() -> Context {
    let root = std::env::temp_dir().join(format!("environment-profiles-{}", store::uuid_v4()));
    Context {
        home_dir: root.join("home"),
        data_dir: root.join("data"),
    }
}
fn call(ctx: &Context, operation: &str, args: Vec<String>) -> agent_manager_engine::Envelope {
    execute(
        ctx,
        &Request {
            command: format!("environment:{operation}"),
            args,
            run_id: "environment-profile-test".into(),
        },
    )
}
fn spec(name: &str) -> Value {
    json!({"name":name,"target":{"timezone":"Europe/London","region":"GB","time_synchronized":true},"auto_sync":false})
}

#[test]
fn builtin_profiles_are_strict_read_only_and_separate_from_saved_copies() {
    let ctx = context();
    let listed = call(&ctx, "profiles", vec![]);
    assert!(listed.ok, "{:?}", listed.errors);
    let builtins = listed.data["builtins"]
        .as_array()
        .expect("built-in profiles");
    assert_eq!(builtins.len(), 2);
    for (preset, timezone, language, region, locale, server) in [
        (
            &builtins[0],
            "America/Los_Angeles",
            "en-US",
            "US",
            "en_US.UTF-8",
            "time.nist.gov",
        ),
        (
            &builtins[1],
            "Asia/Singapore",
            "en-SG",
            "SG",
            "en_US.UTF-8",
            "nets.org.sg",
        ),
    ] {
        assert_eq!(
            preset["target"],
            json!({
                "timezone":timezone,"languages":[language],"region":region,"locale":locale,
                "strict_locale":true,"ntp_server":server,"automatic_time":true,
                "time_synchronized":true,"automatic_timezone":false,"max_clock_offset_ms":2000
            })
        );
        assert!(preset["source_url"]
            .as_str()
            .unwrap()
            .starts_with("https://"));
        assert!(
            !call(
                &ctx,
                "delete-profile",
                vec![
                    "--id".into(),
                    preset["id"].as_str().unwrap().into(),
                    "--base".into(),
                    "0".repeat(64)
                ]
            )
            .ok
        );
        assert!(
            !call(
                &ctx,
                "save-profile",
                vec![
                    "--id".into(),
                    preset["id"].as_str().unwrap().into(),
                    "--base".into(),
                    "0".repeat(64),
                    "--spec".into(),
                    spec("overwrite").to_string()
                ]
            )
            .ok
        );
    }
    assert!(!ctx.home_dir.exists());
    assert!(!ctx.data_dir.exists());
    let copied = call(
        &ctx,
        "save-profile",
        vec![
            "--spec".into(),
            json!({"name":"My Singapore","target":builtins[1]["target"]}).to_string(),
        ],
    );
    assert!(copied.ok, "{:?}", copied.errors);
    let before = std::fs::read(store::db_path(&ctx)).unwrap();
    let after = call(&ctx, "profiles", vec![]);
    assert_eq!(after.data["builtins"], listed.data["builtins"]);
    assert_eq!(after.data["items"].as_array().unwrap().len(), 1);
    let backup = execute(
        &ctx,
        &Request {
            command: "config:export".into(),
            args: vec![],
            run_id: "builtin-backup".into(),
        },
    );
    assert!(backup.ok, "{:?}", backup.errors);
    let saved = backup.data["environment_profiles"].as_array().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0]["id"], copied.data["item"]["id"]);
    assert_eq!(std::fs::read(store::db_path(&ctx)).unwrap(), before);
    std::fs::remove_dir_all(ctx.data_dir.parent().unwrap()).unwrap();
}

#[test]
fn profiles_read_without_creating_state_and_save_reopen_update_delete_with_cas() {
    let ctx = context();
    assert!(call(&ctx, "profiles", vec![]).data["items"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!ctx.data_dir.exists());
    assert!(!ctx.home_dir.exists());
    let saved = call(
        &ctx,
        "save-profile",
        vec!["--spec".into(), spec("London").to_string()],
    );
    assert!(saved.ok, "{:?}", saved.errors);
    let item = &saved.data["item"];
    let id = item["id"].as_str().unwrap();
    let base = item["revision"].as_str().unwrap();
    let before = std::fs::read(store::db_path(&ctx)).unwrap();
    let listed = call(&ctx, "profiles", vec![]);
    assert_eq!(listed.data["items"][0], *item);
    assert_eq!(std::fs::read(store::db_path(&ctx)).unwrap(), before);
    let mut updated = spec("London work");
    updated["target"]["region"] = json!("US");
    let args = vec!["--id".into(), id.into(), "--base".into(), base.into()];
    let mut save_args = args.clone();
    save_args.extend(["--spec".into(), updated.to_string()]);
    let changed = call(&ctx, "save-profile", save_args.clone());
    assert!(changed.ok, "{:?}", changed.errors);
    assert!(!call(&ctx, "save-profile", save_args).ok);
    assert!(!call(&ctx, "delete-profile", args).ok);
    assert_eq!(
        call(&ctx, "profiles", vec![]).data["items"][0]["name"],
        "London work"
    );
    let deleted = call(
        &ctx,
        "delete-profile",
        vec![
            "--id".into(),
            id.into(),
            "--base".into(),
            changed.data["item"]["revision"].as_str().unwrap().into(),
        ],
    );
    assert!(deleted.ok);
    assert!(call(&ctx, "profiles", vec![]).data["items"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!ctx.home_dir.exists());
    std::fs::remove_dir_all(ctx.data_dir.parent().unwrap()).unwrap();
}

#[test]
fn invalid_profiles_and_unconfirmed_clock_writes_never_create_state() {
    for value in [
        json!({"name":"empty","target":{}}),
        json!({"name":"auto","target":{"region":"US"},"auto_sync":true}),
        json!({"name":"bad","target":{"ntp_server":"$(touch injected)"}}),
        json!({"name":"bad","target":{},"password":"do-not-echo"}),
    ] {
        let ctx = context();
        let result = call(
            &ctx,
            "save-profile",
            vec!["--spec".into(), value.to_string()],
        );
        assert!(!result.ok);
        assert!(!ctx.data_dir.exists());
        assert!(!serde_json::to_string(&result.errors)
            .unwrap()
            .contains("do-not-echo"));
    }
    for args in [
        vec![
            "--spec".into(),
            json!({"ntp_server":"time.example.org","max_clock_offset_ms":2000}).to_string(),
        ],
        vec!["--spec".into(), "{}".into(), "--confirm".into()],
        vec![
            "--spec".into(),
            json!({"ntp_server":"-injected","max_clock_offset_ms":2000}).to_string(),
            "--confirm".into(),
        ],
    ] {
        let ctx = context();
        assert!(!call(&ctx, "sync-time", args).ok);
        assert!(!ctx.data_dir.exists());
        assert!(!ctx.home_dir.exists());
    }
}

#[test]
fn duplicate_collection_names_do_not_overwrite_the_saved_target() {
    let ctx = context();
    assert!(
        call(
            &ctx,
            "save-profile",
            vec!["--spec".into(), spec("Example").to_string()]
        )
        .ok
    );
    assert!(
        !call(
            &ctx,
            "save-profile",
            vec!["--spec".into(), spec("example").to_string()]
        )
        .ok
    );
    let items = call(&ctx, "profiles", vec![]).data["items"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Example");
    std::fs::remove_dir_all(ctx.data_dir.parent().unwrap()).unwrap();
}
