//! Real library/child acceptance with a synthetic parent SQLite lock owner.
use super::{fs, open, writer_is_blocked, Fixture};
use agent_manager_engine::{execute, Context, Request};

pub(super) fn run(fixture: &Fixture) {
    agent_manager_engine::configure_claude_data_inspection_helper(Some(
        std::env::var_os("AM_TEST_INSPECTION_HELPER")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_agent-manager-claude-data-helper").into()),
    ));
    let path = fixture.database("inspection-lock-owner.sqlite");
    let before = fs::read(&path).unwrap();
    let dir = fixture.0.join("ai-vault");
    fs::create_dir(&dir).unwrap();
    let alias = dir.join("session-search.sqlite");
    fs::hard_link(&path, &alias).unwrap();
    let db = open(&path);
    db.execute_batch("PRAGMA locking_mode=EXCLUSIVE; BEGIN EXCLUSIVE; UPDATE probe SET value='SYNTHETIC_PENDING';").unwrap();
    writer_is_blocked(&path, true);
    let original = std::env::var_os("ORCA_USER_DATA_PATH");
    std::env::set_var("ORCA_USER_DATA_PATH", &fixture.0);
    let ctx = Context {
        home_dir: fixture.0.clone(),
        data_dir: fixture.0.join("manager-uncreated"),
    };
    let result = execute(
        &ctx,
        &Request {
            command: "claude-data:orca-search-preview".into(),
            args: vec![],
            run_id: "synthetic-isolation".into(),
        },
    );
    match original {
        Some(value) => std::env::set_var("ORCA_USER_DATA_PATH", value),
        None => std::env::remove_var("ORCA_USER_DATA_PATH"),
    }
    assert_eq!(result.data["status"], "unavailable");
    assert!(result.data["counts"].is_null());
    assert!(!ctx.data_dir.exists());
    writer_is_blocked(&path, true);
    db.execute_batch("ROLLBACK;").unwrap();
    db.close().unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::remove_file(alias).unwrap();
    writer_is_blocked(&path, false);
    let selected = dir.join("session-search.sqlite");
    let db = super::Connection::open(&selected).unwrap();
    db.execute_batch(include_str!("../fixtures/orca-search-schema6.sql"))
        .unwrap();
    db.close().unwrap();
    let before = fs::read(&selected).unwrap();
    let original = std::env::var_os("ORCA_USER_DATA_PATH");
    std::env::set_var("ORCA_USER_DATA_PATH", &fixture.0);
    let result = execute(
        &ctx,
        &Request {
            command: "claude-data:orca-search-preview".into(),
            args: vec![],
            run_id: "synthetic-positive".into(),
        },
    );
    match original {
        Some(value) => std::env::set_var("ORCA_USER_DATA_PATH", value),
        None => std::env::remove_var("ORCA_USER_DATA_PATH"),
    }
    assert_eq!(result.data["status"], "inspected");
    assert_eq!(result.data["counts"]["claude_sessions"], 1);
    assert_eq!(result.data["counts"]["non_claude_sessions"], 2);
    assert_eq!(fs::read(selected).unwrap(), before);
    assert!(!ctx.data_dir.exists());
    println!("Isolated source inspection refused a hard-link alias without releasing the parent's SQLite locks or initializing Manager storage.");
}
