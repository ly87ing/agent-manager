-- Synthetic schema-6 layout, checked against Orca 1.4.237's installed scanner.
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
INSERT INTO meta VALUES('schema_version','6'),('index_generation','3');
CREATE TABLE sessions(
 id INTEGER PRIMARY KEY AUTOINCREMENT, agent TEXT NOT NULL,
 session_id TEXT NOT NULL, file_path TEXT NOT NULL, codex_home TEXT,
 title TEXT NOT NULL, cwd TEXT, cwd_key TEXT, branch TEXT, created_at TEXT,
 updated_at TEXT, message_count INTEGER NOT NULL DEFAULT 0,
 resume_command TEXT NOT NULL, content_hash TEXT, content_hash_count INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE files(
 path TEXT PRIMARY KEY, dev INTEGER, ino INTEGER, byte_offset INTEGER NOT NULL,
 mtime_ms REAL NOT NULL, size_bytes INTEGER, session_row_id INTEGER,
 state TEXT NOT NULL DEFAULT 'current', fail_count INTEGER NOT NULL DEFAULT 0,
 failed_mtime_ms REAL
);
CREATE TABLE messages(id INTEGER PRIMARY KEY, session_row_id INTEGER NOT NULL, role TEXT NOT NULL, ts TEXT);
CREATE VIRTUAL TABLE messages_fts USING fts5(
 user_text, assistant_text, tool_text, identifiers, tokenize="unicode61 tokenchars '_.-/+'", detail=full
);
INSERT INTO sessions(id,agent,session_id,file_path,title,resume_command) VALUES
 (1,'claude','same-session-id','/SYNTHETIC/claude.jsonl','SYNTHETIC_CLAUDE_TITLE',''),
 (2,'codex','same-session-id','/SYNTHETIC/codex.jsonl','SYNTHETIC_CODEX_TITLE',''),
 (3,'Claude','different-case','/SYNTHETIC/unknown.jsonl','SYNTHETIC_UNKNOWN_TITLE','');
INSERT INTO messages VALUES(10,1,'user',NULL),(20,2,'user',NULL),(30,99,'user',NULL);
INSERT INTO messages_fts(rowid,user_text) VALUES
 (10,'SYNTHETIC_CLAUDE_BODY'),(20,'SYNTHETIC_CODEX_BODY'),(30,'SYNTHETIC_ORPHAN_BODY'),(40,'SYNTHETIC_FTS_ORPHAN');
INSERT INTO files(path,byte_offset,mtime_ms,session_row_id) VALUES
 ('/SYNTHETIC/claude.jsonl',0,0,1),('/SYNTHETIC/codex.jsonl',0,0,2),
 ('/SYNTHETIC/null.jsonl',0,0,NULL),('/SYNTHETIC/dangling.jsonl',0,0,99);
