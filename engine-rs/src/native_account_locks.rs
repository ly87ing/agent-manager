//! Per-OS-user account locks, independent of a manager's data directory.
//!
//! Never remove or replace a lock inode: another process may still hold it. The
//! legacy bridge holds existing data-directory locks too, but cannot discover an
//! old binary using an arbitrary other data directory. Stop old versions before
//! upgrading; see docs/subsystems/account-context.md for that migration boundary.

use anyhow::{anyhow, bail, Result};
use fs2::FileExt;
use std::fs::{self, File, Metadata, OpenOptions};
use std::path::{Path, PathBuf};

use crate::native::Context;
use crate::platform::Canon;

/// All handles live for the complete child/transaction lifetime. Acquiring a
/// later handle unsuccessfully drops earlier handles, without replacing files.
pub(crate) struct AccountLock {
    // Rust drops fields in declaration order: keep global exclusion until the
    // legacy handles have all been released, including their cohort lease.
    _legacy: Vec<File>,
    _runtime: File,
}

fn resolved(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() || path.components().any(|c| c.as_os_str() == "..") {
        bail!("账号锁需要无父级跳转的绝对路径");
    }
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(path.canon()?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| anyhow!("账号路径无法解析"))?;
            Ok(resolved(parent)?.join(
                path.file_name()
                    .ok_or_else(|| anyhow!("账号路径无法解析"))?,
            ))
        }
        Err(err) => Err(err.into()),
    }
}

fn key(path: &Path) -> String {
    // Match the account-path guard even before a directory has been created.
    // Case-sensitive macOS/Windows volumes conservatively share these locks.
    let name = path.to_string_lossy();
    let name = if cfg!(any(windows, target_os = "macos")) {
        name.to_lowercase()
    } else {
        name.into_owned()
    };
    crate::native_mcp::model::sha256_hex(&name)
}

pub(crate) fn account_key(path: &Path) -> Result<String> {
    Ok(key(&resolved(path)?))
}

#[cfg(unix)]
fn runtime_root() -> Result<PathBuf> {
    // Do not let HOME, TMPDIR or XDG_RUNTIME_DIR split the lock namespace.
    // /tmp itself is an OS alias on macOS and may be canonicalized safely.
    Ok(Path::new("/tmp")
        .canon()?
        .join(format!("agent-manager-account-locks-v1-{}", unsafe {
            libc::geteuid()
        })))
}

#[cfg(windows)]
fn runtime_root() -> Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath},
    };
    let mut raw = std::ptr::null_mut();
    // The OS user profile, not the caller-controlled HOME/TEMP/data directory.
    if unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, 0, std::ptr::null_mut(), &mut raw) }
        < 0
    {
        bail!("无法定位当前 Windows 用户的账号锁目录");
    }
    if raw.is_null() {
        bail!("当前 Windows 用户的账号锁目录为空");
    }
    let path = unsafe {
        let mut len = 0;
        while *raw.add(len) != 0 {
            len += 1;
        }
        let path = PathBuf::from(std::ffi::OsString::from_wide(std::slice::from_raw_parts(
            raw, len,
        )));
        CoTaskMemFree(raw.cast());
        path
    };
    Ok(path.canon()?.join("AgentManager-account-locks-v1"))
}

fn is_link(meta: &Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return meta.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0;
    }
    #[cfg(not(windows))]
    false
}

fn validate_entry(path: &Path, meta: &Metadata, directory: bool, private: bool) -> Result<()> {
    if is_link(meta) || (directory && !meta.is_dir()) || (!directory && !meta.is_file()) {
        bail!(
            "账号锁路径必须是无链接的普通{}，拒绝操作",
            if directory { "目录" } else { "文件" }
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let forbidden = if private { 0o077 } else { 0o022 };
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & forbidden != 0 {
            bail!("账号锁路径的所有者或权限不安全，拒绝操作");
        }
        if !directory && meta.nlink() != 1 {
            bail!("账号锁文件不能含硬链接别名，拒绝操作");
        }
        let _ = path;
    }
    #[cfg(windows)]
    {
        let _ = private;
        crate::secret_values::validate_permissions(path)
            .map_err(|_| anyhow!("账号锁路径的 Windows 所有者或 ACL 不安全，拒绝操作"))?;
        if !directory && file_identity(&File::open(path)?)?.2 != 1 {
            bail!("账号锁文件不能含硬链接别名，拒绝操作");
        }
    }
    Ok(())
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(windows)]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        Security::*,
        Storage::FileSystem::{CreateDirectoryW, FILE_ALL_ACCESS},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut needed = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        if needed == 0 || needed > 64 * 1024 {
            CloseHandle(token);
            return Err(std::io::Error::other("cannot read current Windows user"));
        }
        let mut user = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        let read = GetTokenInformation(
            token,
            TokenUser,
            user.as_mut_ptr().cast(),
            needed,
            &mut needed,
        );
        let error = std::io::Error::last_os_error();
        CloseHandle(token);
        if read == 0 {
            return Err(error);
        }
        let sid = (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        if IsValidSid(sid) == 0 {
            return Err(std::io::Error::other("invalid current Windows user SID"));
        }
        let acl_len = std::mem::size_of::<ACL>() + std::mem::size_of::<ACCESS_ALLOWED_ACE>()
            - std::mem::size_of::<u32>()
            + GetLengthSid(sid) as usize;
        let mut acl_bytes = vec![0u32; acl_len.div_ceil(4)];
        let acl = acl_bytes.as_mut_ptr().cast::<ACL>();
        let mut descriptor: SECURITY_DESCRIPTOR = std::mem::zeroed();
        let descriptor_ptr = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
        if InitializeAcl(acl, (acl_bytes.len() * 4) as u32, ACL_REVISION) == 0
            || AddAccessAllowedAceEx(
                acl,
                ACL_REVISION,
                OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
                FILE_ALL_ACCESS,
                sid,
            ) == 0
            || InitializeSecurityDescriptor(descriptor_ptr, 1) == 0
            || SetSecurityDescriptorDacl(descriptor_ptr, 1, acl, 0) == 0
            || SetSecurityDescriptorControl(descriptor_ptr, SE_DACL_PROTECTED, SE_DACL_PROTECTED)
                == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor_ptr,
            bInheritHandle: 0,
        };
        // The restricted ACL is attached atomically at creation, never after an
        // initially shared directory has become visible to other users.
        if CreateDirectoryW(wide.as_ptr(), &attributes) == 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

fn ensure_directory(path: &Path, private: bool, create: bool) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) => validate_entry(path, &meta, true, private)?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if !create {
                return Ok(false);
            }
            match create_private_directory(path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(err) => return Err(err.into()),
            }
            validate_entry(path, &fs::symlink_metadata(path)?, true, private)?;
        }
        Err(err) => return Err(err.into()),
    }
    Ok(true)
}

#[cfg(windows)]
fn file_identity(file: &File) -> Result<(u32, u64, u32)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let info = unsafe { info.assume_init() };
    Ok((
        info.dwVolumeSerialNumber,
        ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        info.nNumberOfLinks,
    ))
}

fn open_lock(path: &Path, private: bool, create: bool) -> Result<Option<File>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => validate_entry(path, &meta, false, private)?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if !create {
                return Ok(None);
            }
        }
        Err(err) => return Err(err.into()),
    }
    let mut options = OpenOptions::new();
    options.create(create).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    validate_entry(path, &metadata, false, private)?;
    let linked = fs::symlink_metadata(path)?;
    validate_entry(path, &linked, false, private)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (metadata.dev(), metadata.ino()) != (linked.dev(), linked.ino()) {
            bail!("账号锁路径在打开期间发生变化，拒绝操作");
        }
    }
    #[cfg(windows)]
    if file_identity(&file)? != file_identity(&File::open(path)?)? {
        bail!("账号锁路径在打开期间发生变化，拒绝操作");
    }
    Ok(Some(file))
}

fn acquire(file: &File, write: bool, legacy: bool) -> Result<()> {
    // A concurrent fork briefly inherits close-on-exec descriptors. Give those
    // copies time to close, as the manager's existing apply lock does too.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    let result = loop {
        let result = if write {
            file.try_lock_exclusive()
        } else {
            FileExt::try_lock_shared(file)
        };
        if result
            .as_ref()
            .err()
            .is_some_and(|err| err.raw_os_error() == fs2::lock_contended_error().raw_os_error())
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        } else {
            break result;
        }
    };
    result.map_err(|err| {
        if err.raw_os_error() == fs2::lock_contended_error().raw_os_error() {
            if legacy {
                anyhow!("该账号有正在运行的旧版会话或认证/配置操作；请结束旧版会话并退出旧程序，再使用升级后的应用重试")
            } else {
                anyhow!("该账号有正在运行的会话或认证/配置操作，拒绝并发修改")
            }
        } else {
            anyhow!(err).context("无法获取账号会话锁，拒绝操作")
        }
    })
}

fn legacy_directory(home: &Path, data: &Path) -> Result<PathBuf> {
    // HOME is an explicitly trusted root and may have an OS canonical alias.
    // No component below it (including data_dir itself) may redirect old locks.
    let data = if let Ok(relative) = data.strip_prefix(home) {
        resolved(home)?.join(relative)
    } else {
        data.to_path_buf()
    };
    #[cfg(target_os = "macos")]
    let mut data = data;
    #[cfg(target_os = "macos")]
    for trusted in [Path::new("/var"), Path::new("/tmp")] {
        if let Ok(relative) = data.strip_prefix(trusted) {
            data = trusted.canon()?.join(relative);
            break;
        }
    }
    validate_ancestors(&data)?;
    Ok(data.join("account-locks"))
}

fn ancestor_paths(path: &Path) -> Vec<PathBuf> {
    let mut current = PathBuf::new();
    let mut paths = Vec::new();
    for component in path.components() {
        current.push(component);
        // Windows C: alone is drive-relative. Inspecting it would make a lock's
        // safety depend on an unrelated per-drive current working directory.
        if current.is_absolute() {
            paths.push(current.clone());
        }
    }
    paths
}

fn validate_ancestors(path: &Path) -> Result<()> {
    if !path.is_absolute() || path.components().any(|part| part.as_os_str() == "..") {
        bail!("账号锁需要无父级跳转的绝对目录");
    }
    for current in ancestor_paths(path) {
        match fs::symlink_metadata(&current) {
            Ok(meta) if is_link(&meta) || !meta.is_dir() => {
                bail!("账号锁父目录存在链接或非目录路径，拒绝操作");
            }
            Ok(meta) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if ![0, unsafe { libc::geteuid() }].contains(&meta.uid())
                        || (meta.mode() & 0o022 != 0 && meta.mode() & 0o1000 == 0)
                    {
                        bail!("账号锁的父目录可被其他用户替换，请先限制目录权限");
                    }
                }
                #[cfg(windows)]
                {
                    let _ = meta;
                    crate::secret_values::validate_windows_acl(&current, false).map_err(|err| {
                        anyhow!("账号锁父目录的 Windows ACL 不安全，拒绝操作：{err}")
                    })?;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
            Err(err) => return Err(err.into()),
        }
    }
    Ok(())
}

fn known_data_directories(ctx: &Context) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    let base = ctx.home_dir.join("Library/Application Support");
    #[cfg(windows)]
    let base = ctx.home_dir.join("AppData/Roaming");
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = ctx.home_dir.join(".local/share");
    let mut directories = vec![ctx.data_dir.clone()];
    for base in std::iter::once(base).chain(
        #[cfg(windows)]
        std::env::var_os("APPDATA").map(PathBuf::from),
        #[cfg(not(windows))]
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
    ) {
        if base.is_absolute() {
            directories.push(base.join("agent-manager"));
            directories.push(base.join("io.github.ly87ing.agent-manager"));
        }
    }
    directories
}

fn bridge_legacy(runtime: &Path, path: &Path, write: bool) -> Result<Vec<File>> {
    let identity = key(&resolved(path)?);
    let gate = open_lock(&runtime.join(format!("bridge-gate-{identity}")), true, true)?
        .ok_or_else(|| anyhow!("旧版账号锁迁移门不存在"))?;
    // Only a short first-holder probe is serialized, never a whole session.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match gate.try_lock_exclusive() {
            Ok(()) => break,
            Err(err)
                if err.raw_os_error() == fs2::lock_contended_error().raw_os_error()
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(err) => return Err(anyhow!(err).context("账号锁正在迁移，请稍后重试")),
        }
    }
    let Some(file) = open_lock(path, false, false)? else {
        return Ok(Vec::new());
    };
    let lease = open_lock(&runtime.join(format!("bridge-live-{identity}")), true, true)?
        .ok_or_else(|| anyhow!("旧版账号锁迁移租约不存在"))?;
    match lease.try_lock_exclusive() {
        Ok(()) => {
            // A shared legacy lock alone cannot tell old and upgraded holders
            // apart. The first upgraded holder must prove no old session exists.
            acquire(&file, true, true)?;
            if !write {
                // Windows does not convert a LockFileEx exclusive lock into a
                // shared lock. The gate excludes all other upgraded starters.
                #[cfg(windows)]
                FileExt::unlock(&file)?;
                acquire(&file, false, true)?;
            }
            FileExt::unlock(&lease)?;
        }
        Err(err) if err.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            // The gate makes the lease proof stable during this acquisition.
            // Existing upgraded readers have already completed the first probe.
            acquire(&file, write, true)?;
        }
        Err(err) => return Err(err.into()),
    }
    acquire(&lease, false, false)?;
    Ok(vec![file, lease])
}

pub(crate) fn lock_directory(ctx: &Context, dir: &Path, write: bool) -> Result<AccountLock> {
    lock_aliases(ctx, &[dir.to_path_buf()], write)
}

/// Known before/after spellings share one global lock but retain every legacy
/// hash. This matters for case-insensitive paths that do not exist yet.
pub(crate) fn lock_aliases(ctx: &Context, dirs: &[PathBuf], write: bool) -> Result<AccountLock> {
    let dirs: std::collections::BTreeSet<_> = dirs
        .iter()
        .map(|dir| resolved(dir))
        .collect::<Result<_>>()?;
    let dir = dirs.first().ok_or_else(|| anyhow!("账号锁缺少目录"))?;
    let identity = key(dir);
    if dirs.iter().any(|dir| key(dir) != identity) {
        bail!("不同账号目录不能共用一把运行时锁");
    }
    let runtime = runtime_root()?;
    // A private child alone is insufficient if another user can rename it via
    // a writable parent. This is validation only; never repair OS permissions.
    validate_ancestors(
        runtime
            .parent()
            .ok_or_else(|| anyhow!("账号运行时锁缺少父目录"))?,
    )?;
    ensure_directory(&runtime, true, true)?;
    let file = open_lock(&runtime.join(identity), true, true)?
        .ok_or_else(|| anyhow!("账号运行时锁不存在"))?;
    acquire(&file, write, false)?;

    // Neither reads nor writes initialize legacy locks: only existing inodes
    // need a migration bridge. Keep those unchanged for the entire operation.
    let mut legacy = Vec::new();
    let mut paths = std::collections::BTreeSet::new();
    for data in known_data_directories(ctx) {
        if data != ctx.data_dir {
            // An unused optional default must not constrain the selected data
            // directory. Inspect it only when a legacy directory is present;
            // permission/lookup errors and existing links still fail closed.
            match fs::symlink_metadata(data.join("account-locks")) {
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err.into()),
                Ok(_) => {}
            }
        }
        let directory = legacy_directory(&ctx.home_dir, &data)?;
        if !ensure_directory(&directory, false, false)? {
            continue;
        }
        for dir in &dirs {
            let name = crate::native_mcp::model::sha256_hex(&dir.to_string_lossy());
            let path = directory.join(name);
            if paths.insert(key(&resolved(&path)?)) {
                legacy.extend(bridge_legacy(&runtime, &path, write)?);
            }
        }
    }
    Ok(AccountLock {
        _legacy: legacy,
        _runtime: file,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    struct Fixture {
        root: PathBuf,
        ctx: Context,
        account: PathBuf,
    }

    impl Fixture {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().canon().unwrap().join(format!(
                "am-lock-{tag}-{}-{:x}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            create_private_directory(&root).unwrap();
            let ctx = Context {
                home_dir: root.join("home"),
                data_dir: root.join("data-a"),
            };
            fs::create_dir(&ctx.home_dir).unwrap();
            let account = ctx.home_dir.join("not-created/account");
            Self { root, ctx, account }
        }
        fn other(&self) -> Context {
            Context {
                home_dir: self.ctx.home_dir.clone(),
                data_dir: self.root.join("data-b"),
            }
        }
        fn legacy(&self, ctx: &Context) -> (PathBuf, File) {
            fs::create_dir_all(&ctx.data_dir).unwrap();
            let directory = ctx.data_dir.join("account-locks");
            fs::create_dir_all(&directory).unwrap();
            let name = crate::native_mcp::model::sha256_hex(
                &resolved(&self.account).unwrap().to_string_lossy(),
            );
            let path = directory.join(name);
            let file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            (path, file)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    struct Session(Child);
    impl Drop for Session {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn spawn_session(f: &Fixture, ctx: &Context, mode: &str) -> Session {
        let mut child = Session(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "native_account_locks::tests::account_lock_subprocess",
                    "--nocapture",
                ])
                .env("AM_LOCK_TEST_ROOT", &f.root)
                .env("AM_LOCK_TEST_DATA", &ctx.data_dir)
                .env("AM_LOCK_TEST_MODE", mode)
                .env("HOME", f.root.join("different-home"))
                .env("TMPDIR", f.root.join("different-temp"))
                .env("TEMP", f.root.join("different-temp"))
                .env("TMP", f.root.join("different-temp"))
                .env("XDG_RUNTIME_DIR", f.root.join("different-runtime"))
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(20);
        while !f.root.join("ready").exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "lock child exited before acquiring its lock"
            );
            assert!(Instant::now() < deadline, "lock child did not become ready");
            std::thread::sleep(Duration::from_millis(10));
        }
        child
    }
    fn finish_session(f: &Fixture, child: &mut Session) {
        fs::write(f.root.join("continue"), b"continue").unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "lock child did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn account_lock_subprocess() {
        let Some(root) = std::env::var_os("AM_LOCK_TEST_ROOT").map(PathBuf::from) else {
            return;
        };
        let ctx = Context {
            home_dir: root.join("home"),
            data_dir: std::env::var_os("AM_LOCK_TEST_DATA").unwrap().into(),
        };
        let mode = std::env::var("AM_LOCK_TEST_MODE").unwrap();
        let account = ctx.home_dir.join("not-created/account");
        let (_new, _old) = if mode == "legacy" {
            let name = crate::native_mcp::model::sha256_hex(
                &resolved(&account).unwrap().to_string_lossy(),
            );
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(ctx.data_dir.join("account-locks").join(name))
                .unwrap();
            FileExt::try_lock_shared(&file).unwrap();
            (None, Some(file))
        } else {
            (
                Some(lock_directory(&ctx, &account, mode == "write").unwrap()),
                None,
            )
        };
        fs::write(root.join("ready"), b"ready").unwrap();
        let deadline = Instant::now() + Duration::from_secs(25);
        while !root.join("continue").exists() {
            assert!(
                Instant::now() < deadline,
                "parent failed to release fixture"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn cross_process_shared_sessions_exclude_writes_across_data_dirs_and_environment() {
        let f = Fixture::new("cross-data");
        let mut child = spawn_session(&f, &f.ctx, "read");
        let other = f.other();
        // Prove the child alone excludes another data directory before taking
        // any parent read lock that could accidentally mask broken isolation.
        assert!(lock_directory(&other, &f.account, true).is_err());
        let shared = lock_directory(&other, &f.account, false).unwrap();
        assert!(lock_directory(&other, &f.account, true).is_err());
        assert!(lock_directory(&other, &f.ctx.home_dir.join("other-account"), true).is_ok());
        assert!(!f.ctx.data_dir.exists());
        assert!(!other.data_dir.exists());
        assert!(!f.account.exists());
        drop(shared);
        finish_session(&f, &mut child);
        assert!(lock_directory(&other, &f.account, true).is_ok());
    }

    #[test]
    fn cross_process_writer_excludes_readers_and_writers_until_exit() {
        let f = Fixture::new("writer");
        let mut child = spawn_session(&f, &f.ctx, "write");
        for write in [false, true] {
            assert!(lock_directory(&f.other(), &f.account, write).is_err());
        }
        finish_session(&f, &mut child);
        assert!(lock_directory(&f.other(), &f.account, true).is_ok());
    }

    #[test]
    fn readonly_lock_does_not_initialize_existing_or_missing_data_dirs() {
        let f = Fixture::new("read-only");
        assert!(lock_directory(&f.ctx, &f.account, false).is_ok());
        assert!(!f.ctx.data_dir.exists());
        fs::create_dir(&f.ctx.data_dir).unwrap();
        assert!(lock_directory(&f.ctx, &f.account, false).is_ok());
        assert_eq!(fs::read_dir(&f.ctx.data_dir).unwrap().count(), 0);
        assert!(!f.account.exists());
    }

    #[test]
    fn fresh_home_and_failed_legacy_bridge_do_not_create_application_state() {
        let f = Fixture::new("fresh-home");
        fs::remove_dir(&f.ctx.home_dir).unwrap();
        for write in [false, true] {
            assert!(lock_directory(&f.ctx, &f.account, write).is_ok());
            assert!(!f.ctx.home_dir.exists());
            assert!(!f.ctx.data_dir.exists());
        }
        fs::create_dir(&f.ctx.home_dir).unwrap();
        fs::create_dir(&f.ctx.data_dir).unwrap();
        let default = Context {
            home_dir: f.ctx.home_dir.clone(),
            data_dir: known_data_directories(&f.ctx)[1].clone(),
        };
        let (_, legacy) = f.legacy(&default);
        FileExt::try_lock_shared(&legacy).unwrap();
        for write in [false, true] {
            assert!(lock_directory(&f.ctx, &f.account, write).is_err());
            assert_eq!(fs::read_dir(&f.ctx.data_dir).unwrap().count(), 0);
        }
    }

    #[test]
    fn existing_legacy_session_requires_upgrade_restart_and_failure_releases_global_lock() {
        let f = Fixture::new("legacy-old");
        let (path, file) = f.legacy(&f.ctx);
        fs::write(&path, b"legacy-inode-marker").unwrap();
        drop(file);
        let mut child = spawn_session(&f, &f.ctx, "legacy");
        for write in [false, true] {
            let error = lock_directory(&f.ctx, &f.account, write)
                .err()
                .unwrap()
                .to_string();
            assert!(error.contains("结束旧版会话"), "{error}");
            assert_eq!(fs::read(&path).unwrap(), b"legacy-inode-marker");
            // A partial bridge failure must release its new runtime lock.
            if let Err(error) = lock_directory(&f.other(), &f.account, true) {
                panic!("failed bridge retained global lock: {error:#}");
            }
        }
        finish_session(&f, &mut child);
        let a = lock_directory(&f.ctx, &f.account, false).unwrap();
        let b = lock_directory(&f.ctx, &f.account, false).unwrap();
        assert!(lock_directory(&f.other(), &f.account, true).is_err());
        drop(a);
        assert!(lock_directory(&f.other(), &f.account, true).is_err());
        drop(b);
        assert!(lock_directory(&f.ctx, &f.account, true).is_ok());
        assert_eq!(fs::read(path).unwrap(), b"legacy-inode-marker");
    }

    #[test]
    fn existing_global_reader_does_not_vouch_for_old_legacy_reader_in_another_data_dir() {
        let f = Fixture::new("legacy-cohort");
        let other = f.other();
        let (_, legacy) = f.legacy(&other);
        FileExt::try_lock_shared(&legacy).unwrap();
        let running = lock_directory(&f.ctx, &f.account, false).unwrap();
        let error = lock_directory(&other, &f.account, false)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("结束旧版会话"), "{error}");
        drop(legacy);
        let joined = lock_directory(&other, &f.account, false).unwrap();
        let another = lock_directory(&other, &f.account, false).unwrap();
        drop(joined);
        drop(another);
        drop(running);
        assert!(lock_directory(&other, &f.account, true).is_ok());
    }

    #[test]
    fn known_cli_and_desktop_default_legacy_locations_block_custom_data_dirs() {
        let f = Fixture::new("legacy-defaults");
        let defaults = known_data_directories(&f.ctx);
        for data in defaults.into_iter().skip(1).take(2) {
            let ctx = Context {
                home_dir: f.ctx.home_dir.clone(),
                data_dir: data,
            };
            let (_, legacy) = f.legacy(&ctx);
            FileExt::try_lock_shared(&legacy).unwrap();
            for write in [false, true] {
                let error = lock_directory(&f.ctx, &f.account, write)
                    .err()
                    .unwrap()
                    .to_string();
                assert!(error.contains("结束旧版会话"), "{error}");
                assert!(!f.ctx.data_dir.exists());
            }
            drop(legacy);
            assert!(lock_directory(&f.ctx, &f.account, false).is_ok());
        }
    }

    #[test]
    fn absent_optional_legacy_locations_do_not_restrict_safe_custom_data_dir() {
        let f = Fixture::new("optional-legacy");
        let default = known_data_directories(&f.ctx)[1].clone();
        let first = default
            .strip_prefix(&f.ctx.home_dir)
            .unwrap()
            .components()
            .next()
            .unwrap();
        let outside = f.root.join("unused-default-target");
        fs::create_dir(&outside).unwrap();
        crate::platform::link_dir(&outside, &f.ctx.home_dir.join(first)).unwrap();
        for write in [false, true] {
            assert!(lock_directory(&f.ctx, &f.account, write).is_ok());
            assert!(!f.ctx.data_dir.exists());
            assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        }
        // Once a legacy directory exists through that alias, it must be
        // inspected and rejected, rather than silently missing an old session.
        fs::create_dir_all(default.join("account-locks")).unwrap();
        let error = lock_directory(&f.ctx, &f.account, false)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("链接"), "{error}");
        assert!(!f.ctx.data_dir.exists());
    }

    #[test]
    fn upgraded_legacy_bridge_remains_locked_for_the_whole_session() {
        let f = Fixture::new("legacy-new");
        let (path, old_file) = f.legacy(&f.ctx);
        drop(old_file);
        let mut child = spawn_session(&f, &f.ctx, "read");
        let legacy = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        assert!(legacy.try_lock_exclusive().is_err());
        let shared = lock_directory(&f.ctx, &f.account, false).unwrap();
        assert!(lock_directory(&f.other(), &f.account, true).is_err());
        finish_session(&f, &mut child);
        assert!(legacy.try_lock_exclusive().is_err());
        drop(shared);
        legacy.try_lock_exclusive().unwrap();
    }

    #[test]
    fn existing_ancestor_alias_and_missing_descendants_use_the_same_runtime_lock() {
        let f = Fixture::new("alias");
        let alias = f.root.join("home-alias");
        crate::platform::link_dir(&f.ctx.home_dir, &alias).unwrap();
        let lock = lock_directory(&f.ctx, &f.account, false).unwrap();
        assert!(lock_directory(&f.other(), &alias.join("not-created/account"), true).is_err());
        drop(lock);
        assert!(lock_directory(&f.other(), &alias.join("not-created/account"), true).is_ok());
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn missing_case_aliases_share_locks_and_policy_changes_do_not_self_lock() {
        let mut f = Fixture::new("case-alias");
        let lower = f.ctx.home_dir.join("missing/account");
        let upper = f.ctx.home_dir.join("MISSING/ACCOUNT");
        let lock = lock_directory(&f.ctx, &lower, false).unwrap();
        assert!(lock_directory(&f.other(), &upper, true).is_err());
        drop(lock);
        let old =
            serde_json::json!({"agents":{"definitions":{"a":{"profile":{"config_dir":lower}}}}});
        let new =
            serde_json::json!({"agents":{"definitions":{"a":{"profile":{"config_dir":upper}}}}});
        // Both known legacy spellings must survive global-key deduplication.
        for spelling in [&lower, &upper] {
            f.account = spelling.clone();
            let (_, legacy) = f.legacy(&f.ctx);
            FileExt::try_lock_shared(&legacy).unwrap();
            let error =
                crate::native_account_context::lock_policy_account_changes(&f.ctx, &old, &new)
                    .err()
                    .unwrap()
                    .to_string();
            assert!(error.contains("结束旧版会话"), "{error}");
        }
        let locks =
            crate::native_account_context::lock_policy_account_changes(&f.ctx, &old, &new).unwrap();
        assert_eq!(locks.len(), 1);
        assert!(!lower.exists() && !upper.exists());
    }

    #[test]
    fn legacy_parent_links_are_rejected_without_touching_the_target() {
        let f = Fixture::new("legacy-link");
        let outside = f.root.join("outside");
        fs::create_dir(&outside).unwrap();
        let alias = f.root.join("alias");
        crate::platform::link_dir(&outside, &alias).unwrap();
        let ctx = Context {
            home_dir: f.ctx.home_dir.clone(),
            data_dir: alias.join("nested-data"),
        };
        let error = lock_directory(&ctx, &f.account, false)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("链接"), "{error}");
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
        assert!(lock_directory(&f.ctx, &f.account, true).is_ok());
    }

    #[test]
    fn private_lock_directories_and_files_reject_linked_entries() {
        let f = Fixture::new("private-links");
        let directory = f.root.join("runtime");
        ensure_directory(&directory, true, true).unwrap();
        let alias = f.root.join("runtime-alias");
        crate::platform::link_dir(&directory, &alias).unwrap();
        assert!(ensure_directory(&alias, true, true).is_err());
        let path = directory.join("lock");
        drop(open_lock(&path, true, true).unwrap());
        fs::hard_link(&path, directory.join("hardlink")).unwrap();
        assert!(open_lock(&path, true, false).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn private_windows_lock_acl_is_created_atomically_and_broad_access_is_rejected() {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::{Security::*, Storage::FileSystem::FILE_ALL_ACCESS};
        let f = Fixture::new("windows-acl");
        let directory = f.root.join("runtime");
        ensure_directory(&directory, true, true).unwrap();
        validate_ancestors(&directory).unwrap();
        crate::secret_values::validate_permissions(&directory).unwrap();
        let path = directory.join("lock");
        drop(open_lock(&path, true, true).unwrap());
        crate::secret_values::validate_permissions(&path).unwrap();
        let wide: Vec<u16> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
        // Deliberately weaken only this fresh private fixture's directory ACL.
        // Production must reject it without silently rewriting permissions.
        unsafe {
            let mut world = [0u8; 68];
            let mut len = world.len() as u32;
            assert_ne!(
                CreateWellKnownSid(
                    WinWorldSid,
                    std::ptr::null_mut(),
                    world.as_mut_ptr().cast(),
                    &mut len
                ),
                0
            );
            let mut acl = [0u32; 64];
            let acl = acl.as_mut_ptr().cast::<ACL>();
            assert_ne!(InitializeAcl(acl, 256, ACL_REVISION), 0);
            assert_ne!(
                AddAccessAllowedAceEx(
                    acl,
                    ACL_REVISION,
                    0,
                    FILE_ALL_ACCESS,
                    world.as_mut_ptr().cast()
                ),
                0
            );
            let mut descriptor: SECURITY_DESCRIPTOR = std::mem::zeroed();
            let descriptor_ptr = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
            assert_ne!(InitializeSecurityDescriptor(descriptor_ptr, 1), 0);
            assert_ne!(SetSecurityDescriptorDacl(descriptor_ptr, 1, acl, 0), 0);
            assert_ne!(
                SetFileSecurityW(
                    wide.as_ptr(),
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    descriptor_ptr
                ),
                0
            );
        }
        assert!(ensure_directory(&directory, true, false).is_err());
        assert!(validate_ancestors(&directory).is_err());
        assert!(crate::secret_values::validate_windows_acl(&directory, false).is_err());
        assert!(crate::secret_values::validate_permissions(&directory).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn private_root_and_files_reject_permissions_symlinks_hardlinks_and_wrong_owner() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let f = Fixture::new("permissions");
        let directory = f.root.join("runtime");
        ensure_directory(&directory, true, true).unwrap();
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let path = directory.join("lock");
        drop(open_lock(&path, true, true).unwrap());
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(open_lock(&path, true, false).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let second = directory.join("hardlink");
        fs::hard_link(&path, &second).unwrap();
        assert!(open_lock(&path, true, false).is_err());
        fs::remove_file(second).unwrap();
        let alias = directory.join("alias");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(open_lock(&alias, true, true).is_err());
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(ensure_directory(&directory, true, true).is_err());
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        // / normally belongs to another uid in unprivileged test environments.
        let foreign = fs::metadata("/").unwrap();
        if foreign.uid() != unsafe { libc::geteuid() } {
            assert!(validate_entry(Path::new("/"), &foreign, true, false).is_err());
        }
        assert!(open_lock(&path, true, false).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn writable_legacy_ancestors_are_rejected_without_changing_files_or_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new("legacy-permissions");
        let (path, file) = f.legacy(&f.ctx);
        drop(file);
        fs::write(&path, b"unchanged").unwrap();
        fs::set_permissions(&f.ctx.data_dir, fs::Permissions::from_mode(0o777)).unwrap();
        for write in [false, true] {
            let error = lock_directory(&f.ctx, &f.account, write)
                .err()
                .unwrap()
                .to_string();
            assert!(error.contains("父目录可被其他用户替换"), "{error}");
            assert_eq!(fs::read(&path).unwrap(), b"unchanged");
            assert_eq!(
                fs::metadata(&f.ctx.data_dir).unwrap().permissions().mode() & 0o777,
                0o777
            );
        }
        fs::set_permissions(&f.ctx.data_dir, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(lock_directory(&f.ctx, &f.account, true).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn runtime_ancestors_reject_replacement_permissions_but_allow_trusted_sticky_temp() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new("runtime-parent");
        let parent = f.root.join("runtime-parent");
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(validate_ancestors(&parent).is_err());
        assert_eq!(
            fs::metadata(&parent).unwrap().permissions().mode() & 0o7777,
            0o777
        );
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o1777)).unwrap();
        assert!(validate_ancestors(&parent).is_ok());
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(validate_ancestors(&parent).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn windows_ancestor_validation_never_inspects_a_drive_relative_prefix() {
        for path in [r"C:\Users\fixture\data", r"\\server\share\fixture\data"] {
            let paths = ancestor_paths(Path::new(path));
            assert!(!paths.is_empty());
            assert!(paths.iter().all(|path| path.is_absolute()), "{paths:?}");
            assert!(
                !paths.iter().any(|path| path == Path::new("C:")),
                "{paths:?}"
            );
            assert_eq!(paths.last().unwrap(), Path::new(path));
        }
    }

    #[cfg(unix)]
    #[test]
    fn dangling_account_alias_is_not_treated_as_a_new_directory() {
        let f = Fixture::new("dangling");
        let path = f.ctx.home_dir.join("dangling");
        std::os::unix::fs::symlink(f.root.join("missing-target"), &path).unwrap();
        assert!(lock_directory(&f.ctx, &path.join("account"), false).is_err());
        assert!(!f.ctx.data_dir.exists());
    }
}
