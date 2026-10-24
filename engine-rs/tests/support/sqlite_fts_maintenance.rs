//! Synthetic compaction proof only; not a production cleanup or VFS implementation.
use super::{fs, open, writer_is_blocked, Connection, Fixture, OpenFlags};
use rusqlite::types::ValueRef;
use std::path::Path;

const TARGET: &str = "SYNTHETIC_CLAUDE_BODY";
const INDEX_TOKEN: &str = "amclauderesidualtoken7f8ac2";
const DELETE: &str = "
BEGIN EXCLUSIVE;
DELETE FROM messages_fts WHERE rowid IN (
 SELECT m.id FROM messages m JOIN sessions s ON s.id=m.session_row_id
 WHERE typeof(m.session_row_id)='integer' AND s.agent='claude' COLLATE BINARY
);
DELETE FROM messages WHERE id IN (
 SELECT m.id FROM messages m JOIN sessions s ON s.id=m.session_row_id
 WHERE typeof(m.session_row_id)='integer' AND s.agent='claude' COLLATE BINARY
);
DELETE FROM files WHERE path IN (
 SELECT f.path FROM files f JOIN sessions s ON s.id=f.session_row_id
 WHERE typeof(f.session_row_id)='integer' AND s.agent='claude' COLLATE BINARY
);
DELETE FROM sessions WHERE agent='claude' COLLATE BINARY;
";

// Preserve SQLite storage classes, integer values, floating bits and exact text/blob
// bytes. JSON number conversion would not establish this preservation property.
type Rows = Vec<Vec<(u8, Vec<u8>)>>;
fn rows(db: &Connection, sql: &str) -> Rows {
    let mut statement = db.prepare(sql).unwrap();
    let count = statement.column_count();
    statement
        .query_map([], |row| {
            (0..count)
                .map(|column| {
                    Ok(match row.get_ref(column)? {
                        ValueRef::Null => (0, vec![]),
                        ValueRef::Integer(v) => (1, v.to_le_bytes().to_vec()),
                        ValueRef::Real(v) => (2, v.to_bits().to_le_bytes().to_vec()),
                        ValueRef::Text(v) => (3, v.to_vec()),
                        ValueRef::Blob(v) => (4, v.to_vec()),
                    })
                })
                .collect()
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn retained(db: &Connection) -> Vec<Rows> {
    db.execute_batch("CREATE VIRTUAL TABLE IF NOT EXISTS temp.retained_vocab USING fts5vocab(main, messages_fts, instance);").unwrap();
    [
        "SELECT * FROM sessions WHERE id!=1 ORDER BY id",
        "SELECT * FROM messages WHERE id!=10 ORDER BY id",
        "SELECT * FROM files WHERE path!='/SYNTHETIC/claude.jsonl' ORDER BY path COLLATE BINARY",
        "SELECT rowid,* FROM messages_fts WHERE rowid!=10 ORDER BY rowid",
        "SELECT term,doc,col,offset FROM temp.retained_vocab WHERE doc!=10 ORDER BY term COLLATE BINARY,doc,col COLLATE BINARY,offset",
        "SELECT * FROM meta WHERE key!='index_generation' ORDER BY key COLLATE BINARY",
        "SELECT * FROM sqlite_sequence ORDER BY name COLLATE BINARY",
        "SELECT k,v FROM messages_fts_config ORDER BY k COLLATE BINARY",
        "SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type COLLATE BINARY,name COLLATE BINARY",
    ].map(|sql| rows(db, sql)).to_vec()
}

fn fixture(fixture: &Fixture, name: &str) -> std::path::PathBuf {
    let path = fixture.0.join(name);
    let db = Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    db.execute_batch(
        &include_str!("../fixtures/orca-search-schema6.sql")
            .replace(TARGET, &format!("{TARGET} {INDEX_TOKEN}")),
    )
    .unwrap();
    db.execute_batch(include_str!(
        "../fixtures/orca-search-schema6-maintenance.sql"
    ))
    .unwrap();
    db.execute_batch("INSERT INTO meta VALUES('index_incarnation','SYNTHETIC_KEEP_INCARNATION');
      UPDATE sqlite_sequence SET seq=9007199254741001 WHERE name='sessions';
      UPDATE sessions SET title='SYNTHETIC_CODEX_TITLE mentions claude',content_hash=x'0041ff',created_at=NULL,updated_at='2026-01-01',message_count=9223372036854775807 WHERE id=2;
      UPDATE files SET dev=9223372036854775807,ino=-1,mtime_ms=0.125,failed_mtime_ms=NULL WHERE path='/SYNTHETIC/codex.jsonl';
      UPDATE messages_fts SET user_text='SYNTHETIC_CODEX_BODY mentions claude repeat repeat',assistant_text='a.b/path+value',tool_text='SYNTHETIC_KEEP_TOOL',identifiers='keep_identifier' WHERE rowid=20;").unwrap();
    db.close().unwrap();
    assert!(contains(&path, TARGET));
    path
}

fn contains(path: &Path, text: &str) -> bool {
    fs::read(path)
        .unwrap()
        .windows(text.len())
        .any(|bytes| bytes == text.as_bytes())
}

fn checkpoint(db: &Connection) {
    let result: (i64, i64, i64) = db
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert_eq!(result, (0, 0, 0));
}

fn index_contains(db: &Connection) -> bool {
    db.prepare("SELECT block FROM messages_fts_data")
        .unwrap()
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .unwrap()
        .any(|row| {
            row.unwrap()
                .windows(INDEX_TOKEN.len())
                .any(|v| v == INDEX_TOKEN.as_bytes())
        })
}

fn selected_rows_absent(db: &Connection) {
    for sql in [
        "SELECT count(*) FROM sessions WHERE id=1 OR agent='claude' COLLATE BINARY",
        "SELECT count(*) FROM messages WHERE id=10 OR (typeof(session_row_id)='integer' AND session_row_id=1)",
        "SELECT count(*) FROM files WHERE path='/SYNTHETIC/claude.jsonl' OR (typeof(session_row_id)='integer' AND session_row_id=1)",
        "SELECT count(*) FROM messages_fts WHERE rowid=10",
    ] {
        let count: i64 = db.query_row(sql, [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0, "Selected rows must be absent: {sql}");
    }
}

fn compact(db: &Connection, path: &Path) {
    db.execute_batch("INSERT INTO messages_fts(messages_fts,rank) VALUES('integrity-check',1);")
        .unwrap();
    writer_is_blocked(path, true);
    db.execute_batch("VACUUM;").unwrap();
    writer_is_blocked(path, true);
    checkpoint(db);
    writer_is_blocked(path, true);
}

fn delete_owned(db: &Connection, rebuild: bool) {
    db.execute_batch(DELETE).unwrap();
    if rebuild {
        db.execute_batch("INSERT INTO messages_fts(messages_fts) VALUES('rebuild');")
            .unwrap();
    }
    db.execute_batch("COMMIT;").unwrap();
    selected_rows_absent(db);
}

fn verify(path: &Path, before: &[Rows], generation: i64) {
    // The maintenance connection has closed before an independent connection/raw
    // read opens the source. It cannot cancel maintenance's POSIX locks.
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .unwrap();
    assert_eq!(retained(&db), before);
    selected_rows_absent(&db);
    assert!(
        !index_contains(&db),
        "Old target token must leave FTS data blocks"
    );
    let current: i64 = db
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key='index_generation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        current,
        generation + 1,
        "Only the selected cursor deletion increments generation"
    );
    let integrity: String = db
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    db.close().unwrap();
    assert!(!contains(path, TARGET));
    assert!(!contains(path, INDEX_TOKEN));
    assert!(contains(path, "SYNTHETIC_CODEX_BODY"));
}

fn vacuum_only_control(fixture_root: &Fixture) {
    // VACUUM alone removes freed content cells but preserves tombstoned FTS
    // segments. Check the lowercase token in the actual index blocks too.
    let path = fixture(fixture_root, "fts-vacuum-only.sqlite");
    let db = open(&path);
    db.execute_batch("PRAGMA locking_mode=EXCLUSIVE; PRAGMA temp_store=MEMORY;")
        .unwrap();
    assert!(index_contains(&db));
    delete_owned(&db, false);
    db.execute_batch("VACUUM;").unwrap();
    checkpoint(&db);
    assert!(
        index_contains(&db),
        "VACUUM without rebuild must retain the old index token"
    );
    db.close().unwrap();
    assert!(!contains(&path, TARGET));
    assert!(contains(&path, INDEX_TOKEN));
}

pub(super) fn run(fixture_root: &Fixture) {
    vacuum_only_control(fixture_root);
    for (name, resume_after_logical_delete) in
        [("fts-complete.sqlite", false), ("fts-retry.sqlite", true)]
    {
        let path = fixture(fixture_root, name);
        let held = fs::File::open(&path).unwrap();
        let db = open(&path);
        db.execute_batch("PRAGMA locking_mode=EXCLUSIVE; PRAGMA temp_store=MEMORY;")
            .unwrap();
        let before = retained(&db);
        let generation: i64 = db
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key='index_generation'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        delete_owned(&db, !resume_after_logical_delete);
        writer_is_blocked(&path, true);
        if resume_after_logical_delete {
            checkpoint(&db);
            db.close().unwrap();
            drop(held);
            assert!(
                contains(&path, TARGET),
                "The negative control must retain old FTS content"
            );
            let db = open(&path);
            db.execute_batch(
                "PRAGMA locking_mode=EXCLUSIVE; PRAGMA temp_store=MEMORY; BEGIN EXCLUSIVE; COMMIT;",
            )
            .unwrap();
            selected_rows_absent(&db);
            assert!(index_contains(&db));
            delete_owned(&db, true);
            compact(&db, &path);
            db.close().unwrap();
        } else {
            compact(&db, &path);
            db.close().unwrap();
            drop(held);
        }
        verify(&path, &before, generation);
        writer_is_blocked(&path, false);
        let db = open(&path);
        let written: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            written, 17,
            "The writer control must actually write after maintenance closes"
        );
        db.close().unwrap();
    }
    println!("Synthetic schema-6 FTS compaction and zero-owned-row retry preserved typed non-target data, schema, metadata and token positions.");
}
