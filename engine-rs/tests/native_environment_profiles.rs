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
