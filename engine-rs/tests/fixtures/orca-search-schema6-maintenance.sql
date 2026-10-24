-- Synthetic reproduction of Orca 1.4.237's schema-6 indexes and triggers.
CREATE VIRTUAL TABLE messages_vocab USING fts5vocab(messages_fts, 'row');
CREATE INDEX sessions_agent ON sessions(agent);
CREATE INDEX sessions_updated_at ON sessions(updated_at);
CREATE INDEX sessions_cwd_key ON sessions(cwd_key);
CREATE INDEX files_mtime ON files(mtime_ms);
CREATE INDEX messages_session ON messages(session_row_id);
CREATE TRIGGER search_generation_file_insert AFTER INSERT ON files BEGIN
 INSERT INTO meta(key,value) VALUES('index_generation','1')
 ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1;
END;
CREATE TRIGGER search_generation_file_update AFTER UPDATE ON files BEGIN
 INSERT INTO meta(key,value) VALUES('index_generation','1')
 ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1;
END;
CREATE TRIGGER search_generation_file_delete AFTER DELETE ON files BEGIN
 INSERT INTO meta(key,value) VALUES('index_generation','1')
 ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1;
END;
CREATE TRIGGER search_generation_orphan_reclaim AFTER DELETE ON messages
WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=OLD.session_row_id) BEGIN
 INSERT INTO meta(key,value) VALUES('index_generation','1')
 ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1;
END;
