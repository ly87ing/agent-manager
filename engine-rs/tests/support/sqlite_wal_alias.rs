//! Synthetic default-VFS fault control; never a production cleanup path.
use super::{connection_identity, file_identity, fs, open, writer_is_blocked, CStr, Fixture};

pub(super) fn run(fixture: &Fixture) {
    let path = fixture.database("vfs-alias-close.sqlite");
    let held = fs::File::open(&path).unwrap();
    let identity = file_identity(&held);
    let db = open(&path);
    db.execute_batch("PRAGMA locking_mode=EXCLUSIVE; BEGIN EXCLUSIVE; COMMIT;")
        .unwrap();
    writer_is_blocked(&path, true);
    let alias = std::path::PathBuf::from(format!("{}-wal", path.display()));
    let previous = fixture.0.join("retained-original-wal");
    fs::rename(&alias, &previous).unwrap();
    fs::hard_link(&path, &alias).unwrap();
    close_as_wal(&path, &alias, &identity);
    // Restore the exact binding before proving that identity alone misses the
    // lost lock. No subsequent parent SQL write is attempted.
    fs::remove_file(&alias).unwrap();
    fs::rename(previous, alias).unwrap();
    assert_eq!(connection_identity(&db), identity);
    writer_is_blocked(&path, false);
    drop(db);
    drop(held);
    println!("Actual default-VFS WAL alias xClose released source locks despite restored single-link identity.");
}

fn close_as_wal(path: &std::path::Path, alias: &std::path::Path, identity: &[u64]) {
    use rusqlite::ffi::*;
    use std::ffi::CString;
    unsafe {
        let vfs = sqlite3_vfs_find(std::ptr::null());
        assert!(!vfs.is_null());
        let file = sqlite3_malloc64((*vfs).szOsFile as u64).cast::<sqlite3_file>();
        assert!(!file.is_null());
        std::ptr::write_bytes(file.cast::<u8>(), 0, (*vfs).szOsFile as usize);
        let main = CString::new(path.to_str().unwrap()).unwrap();
        let journal = CString::new(format!("{}-journal", path.display())).unwrap();
        let wal = CString::new(alias.to_str().unwrap()).unwrap();
        // Use SQLite's public allocation for the VFS filename format.
        let filename = sqlite3_create_filename(
            main.as_ptr(),
            journal.as_ptr(),
            wal.as_ptr(),
            0,
            std::ptr::null_mut(),
        );
        assert!(!filename.is_null());
        let mut out_flags = 0;
        let rc = (*vfs).xOpen.unwrap()(
            vfs,
            sqlite3_filename_wal(filename),
            file,
            SQLITE_OPEN_READWRITE | SQLITE_OPEN_WAL | SQLITE_OPEN_NOFOLLOW,
            &mut out_flags,
        );
        assert_eq!(rc, SQLITE_OK);
        assert_ne!(out_flags & SQLITE_OPEN_READWRITE, 0);
        let builder = sqlite3_str_new(std::ptr::null_mut());
        assert!(!builder.is_null());
        assert_eq!(
            (*(*file).pMethods).xFileControl.unwrap()(file, SQLITE_FCNTL_FILESTAT, builder.cast()),
            SQLITE_OK
        );
        assert_eq!(sqlite3_str_errcode(builder), SQLITE_OK);
        assert!((1..=16384).contains(&sqlite3_str_length(builder)));
        let value = sqlite3_str_finish(builder);
        assert!(!value.is_null());
        let data: serde_json::Value =
            serde_json::from_slice(CStr::from_ptr(value).to_bytes()).unwrap();
        sqlite3_free(value.cast());
        let fd: i32 = data["h"].as_i64().unwrap().try_into().unwrap();
        assert!(fd >= 0);
        let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
        assert_eq!(libc::fstat(fd, info.as_mut_ptr()), 0);
        let info = info.assume_init();
        assert_eq!(
            [info.st_dev as u64, info.st_ino as u64].as_slice(),
            identity
        );
        assert_eq!(info.st_nlink, 2);
        assert_eq!(info.st_mode & libc::S_IFMT, libc::S_IFREG);
        writer_is_blocked(&path, true);
        assert_eq!((*(*file).pMethods).xClose.unwrap()(file), SQLITE_OK);
        sqlite3_free(file.cast());
        sqlite3_free_filename(filename);
    }
}
