//! Retired collection commands fail before reading sessions or initializing state.

#[cfg(windows)]
mod common;

use agent_manager_engine::platform::Canon;
use agent_manager_engine::{execute, store, Context, Request};
use std::{fs, path::PathBuf};

fn fixture(tag: &str) -> (PathBuf, Context) {
    let root = std::env::temp_dir().canon().unwrap().join(format!(
        "am-retired-usage-{}-{tag}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    #[cfg(windows)]
    common::private_fixture_root(&root);
    #[cfg(not(windows))]
    fs::create_dir(&root).unwrap();
    let ctx = Context {
        home_dir: root.join("home"),
        data_dir: root.join("data"),
    };
    (root, ctx)
}

fn reject_all(ctx: &Context) {
    for command in [
        "usage:status",
        "usage:scan",
        "usage:clear",
        "usage:sessions",
        "usage.scan",
    ] {
        let result = execute(
            ctx,
            &Request {
                command: command.into(),
                args: vec![],
                run_id: "retired-usage".into(),
            },
        );
        assert!(!result.ok, "{command} must be retired");
        assert_eq!(
            result.errors[0].code, "FEATURE_RETIRED",
            "{command}: {:?}",
            result.errors
        );
        assert!(result.data.as_object().unwrap().is_empty());
    }
}

#[test]
fn retired_commands_do_not_create_home_or_database() {
    let (root, ctx) = fixture("fresh");
    reject_all(&ctx);
    assert!(!ctx.home_dir.exists());
    assert!(!ctx.data_dir.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retired_commands_preserve_existing_statistics_and_sessions() {
    let (root, ctx) = fixture("existing");
    fs::create_dir_all(ctx.home_dir.join(".codex/sessions")).unwrap();
    let session = ctx.home_dir.join(".codex/sessions/rollout-fixture.jsonl");
    fs::write(&session, "existing session contents\n").unwrap();
    let connection = store::open_write(&ctx).unwrap();
    connection.execute_batch("CREATE TABLE usage_hourly (
        hour TEXT, agent TEXT, model TEXT, input INTEGER, output INTEGER,
        cache_write INTEGER, cache_read INTEGER, requests INTEGER);
        INSERT INTO usage_hourly VALUES ('2026-10-10T00:00:00Z', 'codex', 'fixture', 1, 2, 3, 4, 5);").unwrap();
    drop(connection);
    let database = ctx.data_dir.join(store::DB_FILE_NAME);
    let before = fs::read(&database).unwrap();
    reject_all(&ctx);
    assert_eq!(fs::read(database).unwrap(), before);
    assert_eq!(
        fs::read_to_string(session).unwrap(),
        "existing session contents\n"
    );
    let connection = store::open_write(&ctx).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT input FROM usage_hourly", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
