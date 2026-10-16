//! Authorization launches only the restricted native helper, never a replayed offset.
use super::time_repair::Adjustment;
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
    sync::OnceLock,
};

static HELPER: OnceLock<PathBuf> = OnceLock::new();
pub(super) fn configure(path: PathBuf) {
    if path.is_absolute() && path.is_file() {
        let _ = HELPER.set(path);
    }
}

struct Prepared {
    path: PathBuf,
    #[cfg(target_os = "linux")]
    root: (PathBuf, u64, u64),
}
impl Drop for Prepared {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            let (root, dev, ino) = &self.root;
            if std::fs::symlink_metadata(root)
                .is_ok_and(|meta| meta.is_dir() && meta.dev() == *dev && meta.ino() == *ino)
            {
                let _ = std::fs::remove_file(&self.path);
                let _ = std::fs::remove_dir(root);
            }
        }
    }
}
fn prepare() -> Option<Prepared> {
    let source = HELPER
        .get()
        .cloned()
        .or_else(|| std::env::current_exe().ok())?;
    prepare_source(&source)
}
fn prepare_source(source: &Path) -> Option<Prepared> {
    let meta = std::fs::symlink_metadata(source).ok()?;
    if !meta.is_file() || meta.len() > 64 * 1024 * 1024 {
        return None;
    }
    #[cfg(windows)]
    {
        Some(Prepared {
            path: source.into(),
        })
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
        // A caller-controlled, non-sticky TMPDIR lets another user rename the
        // private child and replace its executable before pkexec resolves it.
        let parent = dunce::canonicalize("/tmp").ok()?;
        let metadata = std::fs::symlink_metadata(&parent).ok()?;
        if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o1000 == 0 {
            return None;
        }
        let root = parent.join(format!("agent-manager-clock-{}", crate::store::uuid_v4()));
        std::fs::DirBuilder::new().mode(0o700).create(&root).ok()?;
        let meta = std::fs::symlink_metadata(&root).ok()?;
        let mut prepared = Prepared {
            path: root.join("clock-helper"),
            root: (root, meta.dev(), meta.ino()),
        };
        let mut destination = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&prepared.path)
            .ok()?;
        use std::io::Read;
        let mut input = std::fs::File::open(source).ok()?.take(64 * 1024 * 1024 + 1);
        if std::io::copy(&mut input, &mut destination).ok()? > 64 * 1024 * 1024 {
            return None;
        }
        std::io::Write::flush(&mut destination).ok()?;
        // The native engine helper has no GUI-library dependency. A private copy
        // also works when the original sidecar is inside a user-only AppImage FUSE mount.
        prepared.path = dunce::canonicalize(&prepared.path).ok()?;
        Some(prepared)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    #[test]
    fn staging_uses_the_sticky_system_parent_and_preserves_replaced_paths() {
        let root = std::env::temp_dir().join(format!("am-clock-stage-{}", crate::store::uuid_v4()));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("source");
        std::fs::write(&source, b"native-helper-fixture").unwrap();
        let prepared = prepare_source(&source).unwrap();
        let directory = prepared.path.parent().unwrap().to_path_buf();
        assert_eq!(
            directory.parent().unwrap(),
            dunce::canonicalize("/tmp").unwrap()
        );
        assert_eq!(std::fs::metadata(&directory).unwrap().mode() & 0o777, 0o700);
        assert_eq!(
            std::fs::read(&prepared.path).unwrap(),
            b"native-helper-fixture"
        );
        assert_eq!(
            std::fs::metadata(&prepared.path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let old = directory.with_extension("original");
        std::fs::rename(&directory, &old).unwrap();
        std::fs::create_dir(&directory).unwrap();
        let sentinel = directory.join("clock-helper");
        std::fs::write(&sentinel, b"foreign").unwrap();
        drop(prepared);
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"foreign");
        std::fs::remove_dir_all(directory).unwrap();
        std::fs::remove_dir_all(old).unwrap();
        let large = std::fs::File::create(&source).unwrap();
        large.set_len(64 * 1024 * 1024 + 1).unwrap();
        assert!(prepare_source(&source).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(windows)]
fn authorize(executable: &Path, args: &[String]) -> Option<i32> {
    let quote = |value: &str| format!("'{}'", value.replace('\'', "''"));
    let arguments = args
        .iter()
        .map(|value| quote(value))
        .collect::<Vec<_>>()
        .join(",");
    let script=format!("try {{$child=Start-Process -FilePath {} -Verb RunAs -Wait -PassThru -ArgumentList @({arguments}) -ErrorAction Stop; $child.ExitCode}} catch {{if ($_.Exception.NativeErrorCode -eq 1223 -or $_.Exception.InnerException.NativeErrorCode -eq 1223) {{126}} else {{1}}}}",quote(executable.to_str()?));
    super::windows::script_timeout(&script, std::time::Duration::from_secs(150))?
        .parse()
        .ok()
}
#[cfg(target_os = "linux")]
fn authorize(executable: &Path, args: &[String]) -> Option<i32> {
    use std::{process::Stdio, time::Duration};
    let mut command = crate::process::launch_command("/usr/bin/pkexec");
    command
        .arg(executable)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    crate::process::capture_group(&mut command, Duration::from_secs(150), 4096)
        .ok()?
        .status
        .code()
}

pub(super) fn adjust(peer: IpAddr, limit: u32) -> Adjustment {
    super::time_probe::wait_for_interval();
    let Some(helper) = prepare() else {
        return Adjustment::Failed;
    };
    let Some(deadline) = super::clock_helper::boot_ms().and_then(|now| now.checked_add(120_000))
    else {
        return Adjustment::Failed;
    };
    let args = vec![
        super::clock_helper::FLAG.into(),
        peer.to_string(),
        limit.to_string(),
        deadline.to_string(),
        "--confirm".into(),
    ];
    let code = authorize(&helper.path, &args);
    // The helper owns its own UDP client. Wait a complete interval after it
    // returns before the original caller independently measures again.
    super::time_probe::mark_query();
    match code {
        Some(0) => Adjustment::Applied,
        Some(72) => Adjustment::NotNeeded,
        Some(74) => Adjustment::Stale,
        Some(126) => Adjustment::Cancelled,
        _ => Adjustment::Failed,
    }
}
