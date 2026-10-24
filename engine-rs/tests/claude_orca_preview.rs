//! Real CLI acceptance: synthetic mixed store only; no personal Orca paths.
use rusqlite::Connection;
use serde_json::Value;
use std::{fs, path::PathBuf, process::Command};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "am-orca-preview-{}",
            agent_manager_engine::store::uuid_v4()
        ));
        fs::create_dir(&path).unwrap();
        let path = fs::canonicalize(path).unwrap();
        fs::create_dir(path.join("home")).unwrap();
        fs::create_dir_all(path.join("orca/ai-vault")).unwrap();
        Self(path)
    }
    fn database(&self) -> PathBuf {
        self.0.join("orca/ai-vault/session-search.sqlite")
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"));
        command
            .args(["claude-data:orca-search-preview", "--json"])
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("APPDATA", self.0.join("home/AppData/Roaming"))
            .env("LOCALAPPDATA", self.0.join("home/AppData/Local"))
            .env("XDG_CONFIG_HOME", self.0.join("home/.config"))
            .env("AGENT_MANAGER_DATA_DIR", self.0.join("manager"))
            .env("ORCA_USER_DATA_PATH", self.0.join("orca"));
        command
    }
    fn call(&self, extra: &[&str]) -> Value {
        let output = self.command().args(extra).output().unwrap();
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("CLI returned no JSON; status {}", output.status))
    }
    fn populate(&self, wal: bool) {
        let db = Connection::open(self.database()).unwrap();
        if wal {
            db.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        }
        db.execute_batch(include_str!("fixtures/orca-search-schema6.sql"))
            .unwrap();
    }
}

#[test]
fn orca_preview_uses_the_platform_default_for_empty_overrides() {
    let fixture = Fixture::new();
    fixture.populate(false);
    #[cfg(target_os = "macos")]
    let root = fixture.0.join("home/Library/Application Support/orca");
    #[cfg(windows)]
    let root = fixture.0.join("home/AppData/Roaming/orca");
    #[cfg(target_os = "linux")]
    let root = fixture.0.join("home/.config/orca");
    fs::create_dir_all(root.parent().unwrap()).unwrap();
    fs::rename(fixture.0.join("orca"), &root).unwrap();
    let before = fs::read(root.join("ai-vault/session-search.sqlite")).unwrap();
    let output = fixture
        .command()
        .env("ORCA_USER_DATA_PATH", "")
        .env("XDG_CONFIG_HOME", "")
        .output()
        .unwrap();
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["data"]["status"], "inspected", "{response}");
    assert_eq!(response["data"]["location_origin"], "default");
    assert_eq!(response["data"]["counts"]["claude_sessions"], 1);
    assert_eq!(
        fs::read(root.join("ai-vault/session-search.sqlite")).unwrap(),
        before
    );
    assert!(!fixture.0.join("manager").exists());
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn orca_preview_counts_exact_ownership_without_writing_or_exposing_content() {
    for wal in [false, true] {
        let fixture = Fixture::new();
        fixture.populate(wal);
        let before = fs::read(fixture.database()).unwrap();
        if wal {
            assert_eq!(&before[18..20], &[2, 2]);
        }
        let response = fixture.call(&[]);
        assert_eq!(response["ok"], true, "{response}");
        let data = &response["data"];
        assert_eq!(data["status"], "inspected", "{data}");
        assert_eq!(data["counts"]["claude_sessions"], 1);
        assert_eq!(data["counts"]["claude_messages"], 1);
        assert_eq!(data["counts"]["claude_file_cursors"], 1);
        assert_eq!(data["counts"]["non_claude_sessions"], 2);
        assert_eq!(data["counts"]["unattributed_messages"], 1);
        assert_eq!(data["counts"]["unattributed_file_cursors"], 2);
        assert_eq!(data["counts"]["unattributed_fts_rows"], 2);
        assert_eq!(data["cleanup_supported"], false);
        assert_eq!(data["scope_complete"], false);
        let output = response.to_string();
        for secret in [
            "SYNTHETIC_CLAUDE_TITLE",
            "SYNTHETIC_CODEX_BODY",
            "SYNTHETIC_ORPHAN_BODY",
            "same-session-id",
        ] {
            assert!(!output.contains(secret));
        }
        assert_eq!(fs::read(fixture.database()).unwrap(), before);
        assert_eq!(
            fs::read_dir(fixture.database().parent().unwrap())
                .unwrap()
                .count(),
            1
        );
        assert!(!fixture.0.join("manager").exists());
    }
}

#[test]
fn orca_preview_keeps_missing_unsupported_and_busy_stores_explicitly_unchecked() {
    let fixture = Fixture::new();
    assert_eq!(fixture.call(&[])["data"]["status"], "not_found");
    assert!(!fixture.0.join("manager").exists());
    fixture.populate(false);
    let db = Connection::open(fixture.database()).unwrap();
    db.execute_batch("UPDATE meta SET value='7' WHERE key='schema_version';")
        .unwrap();
    drop(db);
    let before = fs::read(fixture.database()).unwrap();
    let data = fixture.call(&[])["data"].clone();
    assert_eq!(data["status"], "unavailable");
    assert_eq!(data["counts"], Value::Null);
    assert_eq!(fs::read(fixture.database()).unwrap(), before);
    let db = Connection::open(fixture.database()).unwrap();
    db.execute_batch("UPDATE meta SET value='6' WHERE key='schema_version';")
        .unwrap();
    drop(db);
    assert_eq!(fixture.call(&[])["data"]["status"], "inspected");
    fs::write(
        fixture.database().with_extension("sqlite-wal"),
        b"SYNTHETIC_PENDING_WAL",
    )
    .unwrap();
    assert_eq!(fixture.call(&[])["data"]["status"], "unavailable");
    assert_eq!(
        fs::read(fixture.database().with_extension("sqlite-wal")).unwrap(),
        b"SYNTHETIC_PENDING_WAL"
    );
    assert_eq!(fixture.call(&["--confirm"])["ok"], false);
    assert!(!fixture.0.join("manager").exists());
}

#[test]
fn orca_preview_refuses_aliases_and_malformed_images_without_changes() {
    let fixture = Fixture::new();
    fixture.populate(false);
    let before = fs::read(fixture.database()).unwrap();
    let alias = fixture.0.join("retained.sqlite");
    fs::hard_link(fixture.database(), &alias).unwrap();
    assert_eq!(fixture.call(&[])["data"]["status"], "unavailable");
    assert_eq!(fs::read(&alias).unwrap(), before);
    fs::remove_file(alias).unwrap();
    fs::write(
        fixture.database(),
        b"not a database SYNTHETIC_PRIVATE_VALUE",
    )
    .unwrap();
    let response = fixture.call(&[]);
    assert_eq!(response["data"]["status"], "unavailable");
    assert!(!response.to_string().contains("SYNTHETIC_PRIVATE_VALUE"));
    assert_eq!(
        fs::read(fixture.database()).unwrap(),
        b"not a database SYNTHETIC_PRIVATE_VALUE"
    );
}

#[test]
fn orca_preview_does_not_guess_noninteger_references_or_accept_views() {
    let fixture = Fixture::new();
    fixture.populate(false);
    let db = Connection::open(fixture.database()).unwrap();
    db.execute_batch("UPDATE messages SET session_row_id=CAST('1' AS BLOB) WHERE id=10; UPDATE files SET session_row_id=CAST('1' AS BLOB) WHERE session_row_id=1;").unwrap();
    drop(db);
    let data = fixture.call(&[])["data"].clone();
    assert_eq!(data["status"], "inspected");
    assert_eq!(data["counts"]["claude_messages"], 0);
    assert_eq!(data["counts"]["claude_file_cursors"], 0);
    assert_eq!(data["counts"]["unattributed_messages"], 2);
    assert_eq!(data["counts"]["unattributed_file_cursors"], 3);
    assert_eq!(data["counts"]["unattributed_fts_rows"], 3);
    let db = Connection::open(fixture.database()).unwrap();
    db.execute_batch("ALTER TABLE sessions RENAME TO retained_sessions; CREATE VIEW sessions AS SELECT * FROM retained_sessions;").unwrap();
    drop(db);
    let before = fs::read(fixture.database()).unwrap();
    assert_eq!(fixture.call(&[])["data"]["status"], "unavailable");
    assert_eq!(fs::read(fixture.database()).unwrap(), before);
}

#[test]
fn orca_preview_refuses_redirected_ancestors_without_touching_the_target() {
    let fixture = Fixture::new();
    fixture.populate(false);
    let before = fs::read(fixture.database()).unwrap();
    fs::rename(fixture.0.join("orca"), fixture.0.join("retained-orca")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(fixture.0.join("retained-orca"), fixture.0.join("orca")).unwrap();
    #[cfg(windows)]
    junction::create(fixture.0.join("retained-orca"), fixture.0.join("orca")).unwrap();
    assert_eq!(fixture.call(&[])["data"]["status"], "unavailable");
    assert_eq!(
        fs::read(
            fixture
                .0
                .join("retained-orca/ai-vault/session-search.sqlite")
        )
        .unwrap(),
        before
    );
    #[cfg(unix)]
    fs::remove_file(fixture.0.join("orca")).unwrap();
    #[cfg(windows)]
    junction::delete(fixture.0.join("orca")).unwrap();
    assert!(!fixture.0.join("manager").exists());
}

#[test]
fn orca_preview_rejects_fake_fts_and_unrecognized_generated_columns() {
    for sql in [
        "DROP TABLE messages_fts; CREATE TABLE messages_fts(user_text,assistant_text,tool_text,identifiers /* using fts5( */);",
        "ALTER TABLE sessions ADD COLUMN hidden_source TEXT GENERATED ALWAYS AS ('SYNTHETIC_UNKNOWN') VIRTUAL;",
    ] {
        let fixture = Fixture::new();
        fixture.populate(false);
        let db = Connection::open(fixture.database()).unwrap();
        db.execute_batch(sql).unwrap();
        drop(db);
        let before = fs::read(fixture.database()).unwrap();
        let data = fixture.call(&[])["data"].clone();
        assert_eq!(data["status"], "unavailable");
        assert_eq!(data["counts"], Value::Null);
        assert_eq!(fs::read(fixture.database()).unwrap(), before);
        assert!(!fixture.0.join("manager").exists());
    }
}
