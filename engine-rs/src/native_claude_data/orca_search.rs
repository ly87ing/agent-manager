//! Read-only inspection of Orca's schema-6 mixed-assistant search index.
//! SQLite sees a bounded in-memory image, never a pathname to the original store.
use super::tree;
use crate::Context;
use anyhow::{bail, Result};
use cap_std::fs::Dir;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

pub(super) const DATABASE: &str = "session-search.sqlite";
const MAX_BYTES: u64 = 256 * 1024 * 1024;

fn location(_ctx: &Context) -> Result<(PathBuf, &'static str)> {
    let (root, origin) =
        if let Some(value) = std::env::var_os("ORCA_USER_DATA_PATH").filter(|v| !v.is_empty()) {
            (PathBuf::from(value), "override")
        } else {
            #[cfg(target_os = "macos")]
            let root = _ctx.home_dir.join("Library/Application Support/orca");
            #[cfg(windows)]
            let root = PathBuf::from(std::env::var_os("APPDATA").ok_or_else(|| {
                anyhow::anyhow!("The Windows application-data location is unavailable.")
            })?)
            .join("orca");
            #[cfg(target_os = "linux")]
            let root = std::env::var_os("XDG_CONFIG_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| _ctx.home_dir.join(".config"))
                .join("orca");
            (root, "default")
        };
    if !root.is_absolute() || root.to_str().is_none() {
        bail!("The Orca user-data location must be absolute and displayable.");
    }
    Ok((root.join("ai-vault").join(DATABASE), origin))
}

pub(super) fn preview(ctx: &Context) -> Result<Value> {
    let (path, origin) = location(ctx)?;
    // All failures are deliberately content-free; SQLite diagnostics can include
    // schema text. A missing default is not evidence about other Orca profiles.
    Ok(match super::inspection_process::inspect(&path) {
        Ok(Some(counts)) => json!({
            "source":"orca_search", "path":path, "location_origin":origin,
            "status":"inspected", "schema_version":6, "counts":counts,
            "cleanup_supported":false, "scope_complete":false,
            "message":"Only this schema-6 search index was inspected. No data was cleaned. Other Orca stores, custom profiles and deleted index content remain unchecked."
        }),
        Ok(None) => json!({
            "source":"orca_search", "path":path, "location_origin":origin,
            "status":"not_found", "schema_version":null, "counts":null,
            "cleanup_supported":false, "scope_complete":false,
            "message":"No search database was found at this location. Other locations and sources remain unchecked."
        }),
        Err(_) => json!({
            "source":"orca_search", "path":path, "location_origin":origin,
            "status":"unavailable", "schema_version":null, "counts":null,
            "cleanup_supported":false, "scope_complete":false,
            "message":"The inspection component or index is unavailable. Check the application installation, close Orca and retry. Redirected paths, sidecars, changed files, unsupported schemas, damaged or oversized databases remain unchecked."
        }),
    })
}

// Keep every ancestor capability and compare its identity after inspection.
// A missing ordinary component means absent; aliases and traversal are refused.
fn parents(path: &Path) -> Result<Option<Vec<(PathBuf, Dir, Vec<u8>)>>> {
    parents_before_open(path, |_, _| Ok(()))
}

fn parents_before_open(
    path: &Path,
    before_open: impl Fn(&Dir, &std::ffi::OsStr) -> Result<()>,
) -> Result<Option<Vec<(PathBuf, Dir, Vec<u8>)>>> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Missing parent"))?;
    let mut anchor = PathBuf::new();
    let mut held: Vec<(PathBuf, Dir, Vec<u8>)> = Vec::new();
    for component in parent.components() {
        match component {
            Component::Prefix(_) => anchor.push(component.as_os_str()),
            Component::RootDir => {
                anchor.push(component.as_os_str());
                let dir = Dir::open_ambient_dir(&anchor, cap_std::ambient_authority())?;
                let identity = tree::identity(&dir)?;
                held.push((anchor.clone(), dir, identity));
            }
            Component::Normal(name) => {
                let dir = &held
                    .last()
                    .ok_or_else(|| anyhow::anyhow!("Relative path"))?
                    .1;
                let metadata = match dir.symlink_metadata(name) {
                    Ok(metadata) => metadata,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(e) => return Err(e.into()),
                };
                ordinary_directory(&metadata)?;
                before_open(dir, name)?;
                let mut options = cap_std::fs::OpenOptions::new();
                options.read(true);
                #[cfg(unix)]
                {
                    use cap_std::fs::OpenOptionsExt;
                    options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
                }
                #[cfg(windows)]
                {
                    use cap_std::fs::OpenOptionsExt;
                    use windows_sys::Win32::Storage::FileSystem::{
                        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
                        FILE_SHARE_WRITE,
                    };
                    // Dir::from_std_file requires directory rename/delete exclusion.
                    options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
                    options.custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
                }
                let next = Dir::from_std_file(dir.open_with(name, &options)?.into_std());
                ordinary_directory(&next.dir_metadata()?)?;
                // Sibling activity changes directory timestamps, length and
                // link count. Bind the no-follow handle below, then rewalk and
                // compare native identities after the database snapshot.
                anchor.push(name);
                let identity = tree::identity(&next)?;
                held.push((anchor.clone(), next, identity));
            }
            _ => bail!("Uncertain path component"),
        }
    }
    Ok(Some(held))
}

fn ordinary_directory(metadata: &cap_std::fs::Metadata) -> Result<()> {
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("Redirected directory");
    }
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            bail!("Reparse directory");
        }
    }
    Ok(())
}

fn no_sidecars(parent: &Dir) -> Result<()> {
    for suffix in ["-wal", "-shm", "-journal"] {
        match parent.symlink_metadata(format!("{DATABASE}{suffix}")) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => bail!("SQLite sidecar exists or cannot be inspected"),
        }
    }
    Ok(())
}

fn image(parent: &Dir) -> Result<Option<Vec<u8>>> {
    let metadata = match parent.symlink_metadata(DATABASE) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES {
        bail!("Unsupported database file");
    }
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            bail!("Reparse database");
        }
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = parent.open_with(DATABASE, &options)?;
    let stamp = tree::stamp(&metadata)?;
    if stamp != tree::stamp(&file.metadata()?)? {
        bail!("Database changed before read");
    }
    super::files::validate_single_link(parent, Path::new(DATABASE))?;
    let mut bytes = Vec::new();
    file.by_ref().take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != metadata.len()
        || stamp != tree::stamp(&file.metadata()?)?
        || stamp != tree::stamp(&parent.symlink_metadata(DATABASE)?)?
    {
        bail!("Database changed during read");
    }
    Ok(Some(bytes))
}

pub(super) fn inspect(path: &Path) -> Result<Option<Value>> {
    inspect_after_read(path, |_| Ok(()))
}

fn inspect_after_read(
    path: &Path,
    after_read: impl FnOnce(&Dir) -> Result<()>,
) -> Result<Option<Value>> {
    let Some(held) = parents(path)? else {
        return Ok(None);
    };
    let parent = &held
        .last()
        .ok_or_else(|| anyhow::anyhow!("Missing parent"))?
        .1;
    no_sidecars(parent)?;
    let Some(bytes) = image(parent)? else {
        return Ok(None);
    };
    let digest = Sha256::digest(&bytes);
    let counts = counts(&bytes)?;
    no_sidecars(parent)?;
    if image(parent)?.as_ref().map(|bytes| Sha256::digest(bytes)) != Some(digest) {
        bail!("Database changed after inspection");
    }
    after_read(parent)?;
    let verified = parents(path)?.ok_or_else(|| anyhow::anyhow!("An ancestor disappeared"))?;
    if verified.len() != held.len()
        || verified
            .iter()
            .zip(&held)
            .any(|(after, before)| after.0 != before.0 || after.2 != before.2)
    {
        bail!("An ancestor was replaced");
    }
    no_sidecars(parent)?;
    Ok(Some(counts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrelated_parent_contents_do_not_change_the_directory_binding() {
        let base =
            std::env::temp_dir().join(format!("am-orca-sibling-{}", crate::store::uuid_v4()));
        std::fs::create_dir_all(base.join("ai-vault")).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let path = base.join("ai-vault").join(DATABASE);
        let db = Connection::open(&path).unwrap();
        db.execute_batch(include_str!("../../tests/fixtures/orca-search-schema6.sql"))
            .unwrap();
        drop(db);
        let before = std::fs::read(&path).unwrap();
        let injected = std::cell::Cell::new(false);
        let result: Result<Value> = (|| {
            let held = parents_before_open(&path, |parent, name| {
                if name == "ai-vault" {
                    #[cfg(unix)]
                    let before = parent.symlink_metadata(name)?;
                    parent.open_dir(name)?.create_dir("unrelated")?;
                    #[cfg(unix)]
                    assert_ne!(
                        tree::stamp(&before)?,
                        tree::stamp(&parent.symlink_metadata(name)?)?
                    );
                    injected.set(true);
                }
                Ok(())
            })?
            .unwrap();
            counts(&image(&held.last().unwrap().1)?.unwrap())
        })();
        assert_eq!(std::fs::read(path).unwrap(), before);
        std::fs::remove_dir_all(base).unwrap();
        assert!(injected.get(), "The sibling mutation was not reached");
        assert_eq!(result.unwrap()["claude_sessions"], 1);
    }

    #[cfg(windows)]
    #[test]
    fn held_ancestors_block_rename_until_the_inspection_releases_them() {
        let base = std::env::temp_dir().join(format!("am-orca-held-{}", crate::store::uuid_v4()));
        std::fs::create_dir_all(base.join("ai-vault")).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let path = base.join("ai-vault");
        std::fs::write(path.join(DATABASE), b"SYNTHETIC_RETAINED_INDEX").unwrap();
        let held = parents(&path.join(DATABASE)).unwrap().unwrap();
        assert!(std::fs::rename(&path, base.join("retained")).is_err());
        assert_eq!(
            std::fs::read(path.join(DATABASE)).unwrap(),
            b"SYNTHETIC_RETAINED_INDEX"
        );
        drop(held);
        std::fs::rename(&path, base.join("retained")).unwrap();
        assert_eq!(
            std::fs::read(base.join("retained").join(DATABASE)).unwrap(),
            b"SYNTHETIC_RETAINED_INDEX"
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_sidecar_created_after_the_second_image_keeps_counts_unavailable() {
        let path = std::env::temp_dir().join(format!("am-orca-late-{}", crate::store::uuid_v4()));
        std::fs::create_dir(&path).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let db = Connection::open(path.join(DATABASE)).unwrap();
        db.execute_batch(include_str!("../../tests/fixtures/orca-search-schema6.sql"))
            .unwrap();
        drop(db);
        let before = std::fs::read(path.join(DATABASE)).unwrap();
        assert!(inspect_after_read(&path.join(DATABASE), |parent| {
            parent.write(format!("{DATABASE}-wal"), b"SYNTHETIC_NEW_WAL")?;
            Ok(())
        })
        .is_err());
        assert_eq!(std::fs::read(path.join(DATABASE)).unwrap(), before);
        assert_eq!(
            std::fs::read(path.join(format!("{DATABASE}-wal"))).unwrap(),
            b"SYNTHETIC_NEW_WAL"
        );
        std::fs::remove_dir_all(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_late_alias_to_the_original_directory_is_not_an_unchanged_ancestor() {
        let base = std::env::temp_dir().join(format!("am-orca-alias-{}", crate::store::uuid_v4()));
        std::fs::create_dir_all(base.join("ai-vault")).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let path = base.join("ai-vault");
        let db = Connection::open(path.join(DATABASE)).unwrap();
        db.execute_batch(include_str!("../../tests/fixtures/orca-search-schema6.sql"))
            .unwrap();
        drop(db);
        let before = std::fs::read(path.join(DATABASE)).unwrap();
        assert!(inspect_after_read(&path.join(DATABASE), |_| {
            std::fs::rename(&path, base.join("retained"))?;
            std::os::unix::fs::symlink(base.join("retained"), &path)?;
            Ok(())
        })
        .is_err());
        assert_eq!(
            std::fs::read(base.join("retained").join(DATABASE)).unwrap(),
            before
        );
        std::fs::remove_dir_all(base).unwrap();
    }
}

fn memory_database(bytes: &[u8]) -> Result<Connection> {
    if bytes.len() < 100
        || &bytes[..16] != b"SQLite format 3\0"
        || !matches!((bytes[18], bytes[19]), (1, 1) | (2, 2))
    {
        bail!("Unsupported SQLite image");
    }
    let db = Connection::open_in_memory()?;
    // SQLite explicitly permits changing these two bytes on the memory image.
    // Allocate its recommended 20-byte overread padding for untrusted images.
    // FREEONCLOSE transfers this sqlite3_malloc64 buffer to SQLite, including
    // error cleanup. N is the source length; M includes the allocated padding.
    let ptr = unsafe { rusqlite::ffi::sqlite3_malloc64((bytes.len() + 20) as u64) }.cast::<u8>();
    let ptr = std::ptr::NonNull::new(ptr).ok_or_else(|| anyhow::anyhow!("Allocation failed"))?;
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.as_ptr(), bytes.len());
        std::ptr::write_bytes(ptr.as_ptr().add(bytes.len()), 0, 20);
        *ptr.as_ptr().add(18) = 1;
        *ptr.as_ptr().add(19) = 1;
        let rc = rusqlite::ffi::sqlite3_deserialize(
            db.handle(),
            c"main".as_ptr(),
            ptr.as_ptr(),
            bytes.len() as i64,
            (bytes.len() + 20) as i64,
            rusqlite::ffi::SQLITE_DESERIALIZE_FREEONCLOSE
                | rusqlite::ffi::SQLITE_DESERIALIZE_READONLY,
        );
        if rc != rusqlite::ffi::SQLITE_OK {
            bail!("SQLite image could not be loaded");
        }
    }
    db.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA query_only=ON; PRAGMA temp_store=MEMORY;")?;
    let start = Instant::now();
    db.progress_handler(
        10_000,
        Some(move || start.elapsed() > Duration::from_secs(3)),
    )?;
    Ok(db)
}

fn counts(bytes: &[u8]) -> Result<Value> {
    let db = memory_database(bytes)?;
    // Validate actual relations before any ownership query; views are not tables.
    for (table, expected) in [
        ("meta", "key,value"),
        ("sessions", "id,agent,session_id,file_path,codex_home,title,cwd,cwd_key,branch,created_at,updated_at,message_count,resume_command,content_hash,content_hash_count"),
        ("files", "path,dev,ino,byte_offset,mtime_ms,size_bytes,session_row_id,state,fail_count,failed_mtime_ms"),
        ("messages", "id,session_row_id,role,ts"),
        ("messages_fts", "user_text,assistant_text,tool_text,identifiers"),
    ] {
        let object: Option<(String,String)> = db.query_row("SELECT type,sql FROM sqlite_schema WHERE name=?1", [table], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((kind, sql)) = object else { bail!("Missing relation") };
        if kind != "table" || (table != "messages_fts" && !sql.to_ascii_uppercase().starts_with("CREATE TABLE ")) {
            bail!("Unsupported relation");
        }
        let table_kind: String = db.query_row("SELECT type FROM pragma_table_list WHERE schema='main' AND name=?1", [table], |r| r.get(0))?;
        if table == "messages_fts" {
            let definition: String = sql.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
            if table_kind != "virtual" || definition.trim_end_matches(';') != "createvirtualtablemessages_ftsusingfts5(user_text,assistant_text,tool_text,identifiers,tokenize=\"unicode61tokenchars'_.-/+'\",detail=full)" {
                bail!("Unsupported search index");
            }
        } else if table_kind != "table" {
            bail!("Unsupported ordinary table");
        }
        let names: Vec<String> = db.prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")?
            .query_map([table], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        if names.join(",") != expected { bail!("Unsupported schema"); }
        let hidden: Vec<String> = db.prepare("SELECT name FROM pragma_table_xinfo(?1) WHERE hidden!=0 ORDER BY cid")?
            .query_map([table], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        if (table == "messages_fts" && hidden != ["messages_fts", "rank"])
            || (table != "messages_fts" && !hidden.is_empty()) { bail!("Unsupported hidden columns"); }
    }
    for (table, name, expected_type, pk) in [
        ("meta", "key", "TEXT", 1),
        ("sessions", "id", "INTEGER", 1),
        ("sessions", "agent", "TEXT", 0),
        ("messages", "id", "INTEGER", 1),
        ("messages", "session_row_id", "INTEGER", 0),
        ("files", "path", "TEXT", 1),
        ("files", "session_row_id", "INTEGER", 0),
    ] {
        let (column_type, primary_key): (String, i64) = db.query_row(
            "SELECT type,pk FROM pragma_table_info(?1) WHERE name=?2",
            [table, name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if column_type.to_ascii_uppercase() != expected_type || primary_key != pk {
            bail!("Unsupported ownership columns");
        }
    }
    for table in ["meta", "sessions", "messages", "files"] {
        let primary_keys: i64 = db.query_row(
            "SELECT count(*) FROM pragma_table_info(?1) WHERE pk>0",
            [table],
            |r| r.get(0),
        )?;
        if primary_keys != 1 {
            bail!("Unsupported composite key");
        }
    }
    let version: String = db.query_row(
        "SELECT value FROM meta WHERE key='schema_version' COLLATE BINARY",
        [],
        |r| r.get(0),
    )?;
    if version != "6" {
        bail!("Unsupported schema version");
    }
    let integrity: String = db.query_row("PRAGMA quick_check(1)", [], |r| r.get(0))?;
    if integrity != "ok" {
        bail!("Damaged database");
    }
    let count = |sql: &str| -> Result<i64> { Ok(db.query_row(sql, [], |r| r.get(0))?) };
    Ok(json!({
        "claude_sessions":count("SELECT count(*) FROM sessions WHERE agent='claude' COLLATE BINARY")?,
        "claude_messages":count("SELECT count(*) FROM messages m JOIN sessions s ON s.id=m.session_row_id WHERE typeof(m.session_row_id)='integer' AND s.agent='claude' COLLATE BINARY")?,
        "claude_file_cursors":count("SELECT count(*) FROM files f JOIN sessions s ON s.id=f.session_row_id WHERE typeof(f.session_row_id)='integer' AND s.agent='claude' COLLATE BINARY")?,
        "non_claude_sessions":count("SELECT count(*) FROM sessions WHERE agent!='claude' COLLATE BINARY OR agent IS NULL")?,
        "unattributed_messages":count("SELECT count(*) FROM messages m WHERE typeof(m.session_row_id)!='integer' OR NOT EXISTS (SELECT 1 FROM sessions s WHERE s.id=m.session_row_id)")?,
        "unattributed_file_cursors":count("SELECT count(*) FROM files f WHERE typeof(f.session_row_id)!='integer' OR NOT EXISTS (SELECT 1 FROM sessions s WHERE s.id=f.session_row_id)")?,
        "unattributed_fts_rows":count("SELECT count(*) FROM messages_fts f WHERE NOT EXISTS (SELECT 1 FROM messages m JOIN sessions s ON s.id=m.session_row_id WHERE m.id=f.rowid AND typeof(m.session_row_id)='integer')")?,
    }))
}
