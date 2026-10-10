//! Account status retains its nullable field but never collects local quota.

#[cfg(windows)]
mod common;

use agent_manager_engine::platform::Canon;
use agent_manager_engine::{execute, Context, Request};
use serde_json::{json, Value};
use std::{fs, path::PathBuf};

fn fixture(tag: &str) -> (PathBuf, Context) {
    let root = std::env::temp_dir().canon().unwrap().join(format!(
        "am-no-quota-{}-{tag}-{}",
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
    fs::create_dir(&ctx.home_dir).unwrap();
    (root, ctx)
}

fn request(command: &str, args: Vec<String>) -> Request {
    Request {
        command: command.into(),
        args,
        run_id: "no-quota".into(),
    }
}

fn define(ctx: &Context, id: &str, definition: Value) {
    let result = execute(
        ctx,
        &request(
            "policy:set",
            vec![
                "--layer".into(),
                "local".into(),
                "--key".into(),
                format!("agents.definitions.{id}"),
                "--value".into(),
                definition.to_string(),
            ],
        ),
    );
    assert!(result.ok, "{:?}", result.errors);
}

fn quota_files(ctx: &Context, claude: &str, codex: &str) -> Vec<(PathBuf, Vec<u8>)> {
    let claude = ctx.home_dir.join(claude);
    let session = ctx
        .home_dir
        .join(codex)
        .join("sessions/rollout-fixture.jsonl");
    fs::create_dir_all(claude.parent().unwrap()).unwrap();
    fs::create_dir_all(session.parent().unwrap()).unwrap();
    let cache = json!({"cachedUsageUtilization": {"fetchedAtMs": 1789605697689_i64,
        "utilization": {"five_hour": {"utilization": 82, "resets_at": "2026-10-11T00:00:00Z"}}}});
    let event = json!({"timestamp": "2026-10-10T00:00:00Z", "payload": {"info": {"rate_limits": {
        "primary": {"used_percent": 82, "window_minutes": 300, "resets_at": 1791676800_i64}}}}});
    fs::write(&claude, cache.to_string()).unwrap();
    fs::write(&session, format!("{event}\n")).unwrap();
    vec![claude, session]
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}

fn assert_no_quota(ctx: &Context, command: &str, ids: &[&str]) {
    let result = execute(ctx, &request(command, vec![]));
    assert!(result.ok, "{:?}", result.errors);
    let elements = result.data["elements"].as_array().unwrap();
    for id in ids {
        let element = elements
            .iter()
            .find(|element| element["id"] == *id)
            .expect("account is present");
        assert_eq!(element["observed"].get("usage"), Some(&Value::Null));
    }
}

#[test]
fn main_status_does_not_collect_existing_quota() {
    let (root, ctx) = fixture("main");
    let files = quota_files(&ctx, ".claude.json", ".codex");
    for (id, bin) in [("claude-code", "claude"), ("codex", "codex")] {
        define(
            &ctx,
            id,
            json!({"_replace": true, "display_name": id, "bin": format!("/missing/{bin}"),
                "preferred_provider": "local-test", "providers": {"local-test": {"install": "true"}}}),
        );
    }
    assert_no_quota(
        &ctx,
        "agents:status",
        &["agents:claude-code", "agents:codex"],
    );
    for (path, before) in files {
        assert_eq!(fs::read(path).unwrap(), before);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn profile_status_does_not_collect_existing_quota() {
    let (root, ctx) = fixture("profile");
    let files = quota_files(&ctx, ".claude-work/.claude.json", ".codex-work");
    for (id, bin, base, dir) in [
        ("claude-work", "claude", "claude-code", "~/.claude-work"),
        ("codex-work", "codex", "codex", "~/.codex-work"),
    ] {
        define(
            &ctx,
            id,
            json!({"display_name": id, "bin": format!("/missing/{bin}"),
            "profile": {"base_agent": base, "kind": bin, "config_dir": dir}}),
        );
    }
    assert_no_quota(
        &ctx,
        "profiles:status",
        &["profiles:claude-work", "profiles:codex-work"],
    );
    for (path, before) in files {
        assert_eq!(fs::read(path).unwrap(), before);
    }
    fs::remove_dir_all(root).unwrap();
}
