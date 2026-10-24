//! Synthetic capability research, not a cleanup implementation or production flag.
//! The diagnostic build uses a separate Cargo target directory in native CI.
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::{
    ffi::CStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

type Identity = Vec<u64>;

#[path = "support/sqlite_fts_maintenance.rs"]
mod fts_maintenance;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "am-sqlite-maintenance-{}",
            agent_manager_engine::store::uuid_v4()
        ));
        fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let path = fs::canonicalize(path).unwrap();
        fs::write(path.join("OWNED_PROOF"), b"SYNTHETIC_ONLY").unwrap();
        Self(path)
    }
    fn database(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE probe(value TEXT); INSERT INTO probe VALUES('baseline');").unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

// This JSON comes from the pinned SQLite C implementation, not session input.
// Production parsing, VFS guards and cleanup authorization are not implemented here.
fn file_stat(db: &Connection) -> Option<Value> {
    unsafe {
        let text = rusqlite::ffi::sqlite3_str_new(db.handle());
        let rc = rusqlite::ffi::sqlite3_file_control(
            db.handle(),
            c"main".as_ptr(),
            rusqlite::ffi::SQLITE_FCNTL_FILESTAT,
            text.cast(),
        );
        let error = rusqlite::ffi::sqlite3_str_errcode(text);
        let size = rusqlite::ffi::sqlite3_str_length(text);
        let result = rusqlite::ffi::sqlite3_str_finish(text);
        let bytes = if result.is_null() {
            Vec::new()
        } else {
            let bytes = CStr::from_ptr(result).to_bytes().to_vec();
            rusqlite::ffi::sqlite3_free(result.cast());
            bytes
        };
        assert_eq!(
            error,
            rusqlite::ffi::SQLITE_OK,
            "SQLite string construction failed"
        );
        if rc == rusqlite::ffi::SQLITE_NOTFOUND {
            assert!(bytes.is_empty());
            return None;
        }
        assert_eq!(rc, rusqlite::ffi::SQLITE_OK, "Unsupported file control");
        assert!((1..=16384).contains(&size));
        assert_eq!(bytes.len(), size as usize);
        Some(serde_json::from_slice(&bytes).expect("Unexpected diagnostic structure"))
    }
}

#[cfg(unix)]
fn file_identity(file: &fs::File) -> Identity {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().unwrap();
    assert!(metadata.is_file());
    assert_eq!(metadata.nlink(), 1);
    vec![metadata.dev(), metadata.ino()]
}

#[cfg(unix)]
fn connection_identity(db: &Connection) -> Identity {
    let value = file_stat(db).expect("FILESTAT is required in the diagnostic build");
    assert_eq!(value["vfs"], "unix");
    let fd: i32 = value["h"]
        .as_i64()
        .expect("Missing root handle")
        .try_into()
        .unwrap();
    assert!(fd >= 0);
    // Borrow only. Closing any additional database FD can release POSIX locks.
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    assert_eq!(unsafe { libc::fstat(fd, stat.as_mut_ptr()) }, 0);
    let stat = unsafe { stat.assume_init() };
    assert_eq!(stat.st_mode & libc::S_IFMT, libc::S_IFREG);
    assert_eq!(stat.st_nlink, 1);
    vec![stat.st_dev as u64, stat.st_ino as u64]
}

#[cfg(windows)]
fn handle_identity(handle: windows_sys::Win32::Foundation::HANDLE) -> Identity {
    use windows_sys::Win32::Storage::FileSystem::*;
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    assert_ne!(
        unsafe { GetFileInformationByHandle(handle, info.as_mut_ptr()) },
        0
    );
    let info = unsafe { info.assume_init() };
    assert_eq!(info.nNumberOfLinks, 1);
    assert_eq!(
        info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT),
        0
    );
    vec![
        info.dwVolumeSerialNumber as u64,
        info.nFileIndexHigh as u64,
        info.nFileIndexLow as u64,
    ]
}

#[cfg(windows)]
fn file_identity(file: &fs::File) -> Identity {
    use std::os::windows::io::AsRawHandle;
    handle_identity(file.as_raw_handle() as _)
}

#[cfg(windows)]
fn connection_identity(db: &Connection) -> Identity {
    let mut handle = std::ptr::null_mut();
    let rc = unsafe {
        rusqlite::ffi::sqlite3_file_control(
            db.handle(),
            c"main".as_ptr(),
            rusqlite::ffi::SQLITE_FCNTL_WIN32_GET_HANDLE,
            (&mut handle as *mut windows_sys::Win32::Foundation::HANDLE).cast(),
        )
    };
    assert_eq!(rc, rusqlite::ffi::SQLITE_OK);
    handle_identity(handle)
}

fn open(path: &Path) -> Connection {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .unwrap()
}

fn writer(path: &Path) -> i32 {
    assert!(path
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("am-sqlite-maintenance-"));
    assert_eq!(
        fs::read(path.parent().unwrap().join("OWNED_PROOF")).unwrap(),
        b"SYNTHETIC_ONLY"
    );
    let db = match Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    ) {
        Ok(db) => db,
        Err(e)
            if matches!(
                e.sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
            ) =>
        {
            return 0
        }
        Err(_) => return 42,
    };
    db.busy_timeout(Duration::ZERO).unwrap();
    // A real header write works for both the minimal and schema-6 fixtures.
    match db.execute_batch("PRAGMA user_version=17;") {
        Ok(_) => 17,
        Err(e)
            if matches!(
                e.sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
            ) =>
        {
            0
        }
        Err(_) => 42,
    }
}

fn writer_is_blocked(path: &Path, blocked: bool) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--writer")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Synthetic SQLite writer exceeded its deadline");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        status.code(),
        Some(if blocked { 0 } else { 17 }),
        "Second-process writer phase"
    );
}

fn builtin_count(db: &Connection) -> i64 {
    db.query_row(
        "SELECT count(*) FROM pragma_function_list WHERE name='sqlite_filestat'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

fn enabled(fixture: &Fixture) {
    let path = fixture.database("proof.sqlite");
    // Keep the extra FD alive until AFTER SQLite closes.
    let held = fs::File::open(&path).unwrap();
    let expected = file_identity(&held);
    let db = open(&path);
    assert_eq!(connection_identity(&db), expected);
    assert!(file_stat(&db).is_some());
    db.execute_batch("PRAGMA locking_mode=EXCLUSIVE; PRAGMA temp_store=MEMORY;")
        .unwrap();
    let mode: String = db
        .query_row("PRAGMA locking_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "exclusive");
    assert_eq!(builtin_count(&db), 1, "The flag also adds a SQL builtin");
    db.execute_batch("BEGIN EXCLUSIVE; INSERT INTO probe VALUES('owned');")
        .unwrap();
    writer_is_blocked(&path, true);
    assert_eq!(connection_identity(&db), expected);
    writer_is_blocked(&path, true);
    db.execute_batch("COMMIT;").unwrap();
    writer_is_blocked(&path, true);
    db.execute_batch("VACUUM;").unwrap();
    writer_is_blocked(&path, true);
    let state: (i64, i64, i64) = db
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(state, (0, 0, 0));
    writer_is_blocked(&path, true);
    assert_eq!(connection_identity(&db), expected);
    writer_is_blocked(&path, true);
    drop(db);
    drop(held);
    writer_is_blocked(&path, false);

    let race = fixture.database("race.sqlite");
    let held = fs::File::open(&race).unwrap();
    let expected = file_identity(&held);
    let original = fs::read(&race).unwrap();
    fs::copy(&race, fixture.0.join("copy.sqlite")).unwrap();
    fs::rename(&race, fixture.0.join("retained-a.sqlite")).unwrap();
    fs::rename(fixture.0.join("copy.sqlite"), &race).unwrap();
    let other = open(&race);
    assert_ne!(connection_identity(&other), expected);
    #[cfg(unix)]
    {
        let mut moved = 1;
        let rc = unsafe {
            rusqlite::ffi::sqlite3_file_control(
                other.handle(),
                c"main".as_ptr(),
                rusqlite::ffi::SQLITE_FCNTL_HAS_MOVED,
                (&mut moved as *mut i32).cast(),
            )
        };
        assert_eq!(rc, rusqlite::ffi::SQLITE_OK);
        assert_eq!(moved, 0, "HAS_MOVED alone misses the baseline mismatch");
        fs::rename(&race, fixture.0.join("retained-b.sqlite")).unwrap();
        fs::rename(fixture.0.join("retained-a.sqlite"), &race).unwrap();
        use std::os::unix::fs::MetadataExt;
        let restored = fs::metadata(&race).unwrap();
        assert_eq!(vec![restored.dev(), restored.ino()], expected);
        assert_ne!(connection_identity(&other), expected);
    }
    #[cfg(windows)]
    {
        let error = fs::rename(&race, fixture.0.join("retained-b.sqlite")).unwrap_err();
        assert_eq!(
            error.raw_os_error(),
            Some(32),
            "Expected native sharing refusal"
        );
        assert_eq!(fs::read(&race).unwrap(), original);
        assert_eq!(
            fs::read(fixture.0.join("retained-a.sqlite")).unwrap(),
            original
        );
    }
    drop(other);
    #[cfg(windows)]
    {
        fs::rename(&race, fixture.0.join("retained-b.sqlite")).unwrap();
        fs::rename(fixture.0.join("retained-a.sqlite"), &race).unwrap();
    }
    assert_eq!(fs::read(&race).unwrap(), original);
    assert_eq!(
        fs::read(fixture.0.join("retained-b.sqlite")).unwrap(),
        original
    );
    drop(held);

    #[cfg(unix)]
    {
        let path = fixture.database("unsafe-close.sqlite");
        let db = open(&path);
        db.execute_batch("PRAGMA locking_mode=EXCLUSIVE; BEGIN EXCLUSIVE; COMMIT;")
            .unwrap();
        writer_is_blocked(&path, true);
        drop(fs::File::open(&path).unwrap());
        writer_is_blocked(&path, false);
        drop(db);
    }
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--writer") {
        assert_eq!(args.len(), 2);
        std::process::exit(writer(Path::new(&args[1])));
    }
    assert_eq!(
        unsafe { CStr::from_ptr(rusqlite::ffi::sqlite3_libversion()) }
            .to_str()
            .unwrap(),
        "3.53.2",
        "Re-review the pinned capability protocol after SQLite upgrades"
    );
    assert_eq!(
        unsafe { CStr::from_ptr(rusqlite::ffi::sqlite3_sourceid()) }
            .to_str()
            .unwrap(),
        "2026-06-03 19:12:13 d6e03d8c777cfa2d35e3b60d8ec3e0187f3e9f99d8e2ee9cac695fd6fcdf1a24",
        "Re-review the exact diagnostic source, not only its version label"
    );
    let fixture = Fixture::new();
    let mode =
        std::env::var("AGENT_MANAGER_SQLITE_PROBE_MODE").unwrap_or_else(|_| "baseline".into());
    if mode == "enabled" {
        enabled(&fixture);
    } else {
        assert_eq!(mode, "baseline");
        let path = fixture.database("baseline.sqlite");
        let before = fs::read(&path).unwrap();
        let db = open(&path);
        assert!(
            file_stat(&db).is_none(),
            "Production compilation unexpectedly enabled diagnostics"
        );
        assert_eq!(builtin_count(&db), 0);
        drop(db);
        assert_eq!(fs::read(path).unwrap(), before);
    }
    fts_maintenance::run(&fixture);
    drop(fixture);
    println!(
        "{}",
        json!({"platform":std::env::consts::OS,"mode":mode,"scope":"synthetic_only","status":"passed"})
    );
}
