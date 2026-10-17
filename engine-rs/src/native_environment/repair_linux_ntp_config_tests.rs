use super::*;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("am-ntp-config-{}", crate::store::uuid_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        let root = Self(std::fs::canonicalize(path).unwrap());
        std::fs::create_dir_all(root.0.join(ROOTS[0])).unwrap();
        root
    }
    fn file(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        path
    }
    fn read(&self) -> Option<State> {
        read_at(&self.0, unsafe { libc::geteuid() })
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn finite_syntax_refuses_sections_and_continuations_that_can_skip_the_owned_file() {
    for text in [
        "",
        "# defaults\n[Time]\nNTP=\nNTP=one.example two.example\n",
        "; defaults\r\n[Time]\r\nFallbackNTP=fallback.example\r\nPollIntervalMinSec=32\r\n",
    ] {
        assert!(admitted(text), "{text:?}");
    }
    for text in [
        "[Time\nNTP=one.example",
        "[Time] trailing\nNTP=one.example",
        "NTP=one.example\n",
        "[Other]\n",
        "[Time]\nUnknown=1\n",
        "# continued\\\n[Time]\n",
        "[Time]\nNTP=one.example\\\ntwo.example\n",
        "[Time]\nNTP=\"one.example\"\n",
        "[Time]\nNTP=https://one.example\n",
        "[Time]\nNTP=one.example\0\n",
    ] {
        assert!(!admitted(text), "{text:?}");
    }
    let expected = generated("Current.Example.").unwrap();
    assert_eq!(saved(&expected).as_deref(), Some("current.example"));
    for text in [
        expected.replace("current.example", "Current.Example"),
        format!("{expected}FallbackNTP=other.example\n"),
        expected.trim_end().to_string(),
    ] {
        assert!(saved(&text).is_none());
    }
}

#[test]
fn read_is_zero_write_and_binds_the_effective_priority_graph() {
    let root = Fixture::new();
    let empty = root.read().unwrap();
    let target = format!("{}/{DROPINS}/{NAME}", ROOTS[0]);
    assert_eq!(empty.saved, None);
    assert_eq!(empty.files[&target], None);
    assert!(!root.0.join(ROOTS[0]).join(DROPINS).exists());
    let upper = root.file(
        "etc/systemd/timesyncd.conf.d/50-vendor.conf",
        "[Time]\nFallbackNTP=upper.example\n",
    );
    let lower = root.file(
        "usr/lib/systemd/timesyncd.conf.d/50-vendor.conf",
        "[Malformed\n",
    );
    let generated = generated("selected.example").unwrap();
    let owned = root.file(&target, &generated);
    let before = root.read().unwrap();
    assert_eq!(before.saved.as_deref(), Some("selected.example"));
    assert!(before
        .files
        .contains_key("etc/systemd/timesyncd.conf.d/50-vendor.conf"));
    assert!(!before
        .files
        .contains_key("usr/lib/systemd/timesyncd.conf.d/50-vendor.conf"));
    assert_eq!(root.read(), Some(before.clone()));
    assert_eq!(std::fs::read_to_string(&owned).unwrap(), generated);
    assert_eq!(std::fs::read_to_string(&lower).unwrap(), "[Malformed\n");
    std::fs::write(&upper, "[Time]\nFallbackNTP=changed.example\n").unwrap();
    assert_ne!(root.read().unwrap(), before);
}

#[test]
fn malformed_prior_files_foreign_targets_shadowing_and_later_files_are_unavailable() {
    let target = format!("{}/{DROPINS}/{NAME}", ROOTS[0]);
    for root_index in 0..ROOTS.len() {
        let root = Fixture::new();
        let invalid = root.file(
            &format!("{}/{DROPINS}/50-invalid.conf", ROOTS[root_index]),
            "[Time\n",
        );
        root.file(&target, &generated("selected.example").unwrap());
        assert!(root.read().is_none());
        std::fs::remove_file(invalid).unwrap();
        assert!(root.read().is_some());
        root.file(
            &format!("{}/{DROPINS}/99-later.conf", ROOTS[root_index]),
            "[Time]\nPollIntervalMinSec=32\n",
        );
        assert!(root.read().is_none());
    }
    for root_index in 1..ROOTS.len() {
        let root = Fixture::new();
        root.file(
            &format!("{}/{DROPINS}/{NAME}", ROOTS[root_index]),
            "[Time]\nFallbackNTP=preserve.example\n",
        );
        assert!(root.read().is_none());
        let root = Fixture::new();
        root.file(&format!("{}/timesyncd.conf", ROOTS[root_index]), "[Time]\n");
        assert!(root.read().is_none());
    }
    for text in ["[Time]\nNTP=foreign.example\n", "", "[Time\n"] {
        let root = Fixture::new();
        root.file(&target, text);
        assert!(root.read().is_none());
    }
    let root = Fixture::new();
    root.file(
        "etc/systemd/timesyncd.conf.d/90-AGENT-MANAGER.CONF",
        "[Time]\n",
    );
    assert!(root.read().is_none());
}

#[test]
fn unsafe_nodes_permissions_and_oversized_inputs_are_not_read_as_safe_configuration() {
    let root = Fixture::new();
    let path = root.file(
        "etc/systemd/timesyncd.conf",
        "[Time]\nNTP=selected.example\n",
    );
    assert!(root.read().is_some());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert!(root.read().is_none());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let hardlink = root.0.join("extra-link");
    std::fs::hard_link(&path, &hardlink).unwrap();
    assert!(root.read().is_none());
    std::fs::remove_file(hardlink).unwrap();
    let real = root.0.join("original");
    std::fs::rename(&path, &real).unwrap();
    std::os::unix::fs::symlink(&real, &path).unwrap();
    assert!(root.read().is_none());
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&real, &path).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(LIMIT as u64 + 1)
        .unwrap();
    assert!(root.read().is_none());
    std::fs::remove_file(path).unwrap();
    let directory = root.0.join(ROOTS[0]);
    let real_directory = root.0.join("original-systemd");
    std::fs::rename(&directory, &real_directory).unwrap();
    std::os::unix::fs::symlink(&real_directory, &directory).unwrap();
    assert!(root.read().is_none());
}

#[test]
fn owned_receipts_project_only_declared_changes_and_do_not_absorb_other_edits() {
    let root = Fixture::new();
    let uid = unsafe { libc::geteuid() };
    let before = root.read().unwrap();
    assert!(before.directory_missing());
    let directory = root.0.join(ROOTS[0]).join(DROPINS);
    std::fs::DirBuilder::new()
        .mode(0o755)
        .create(&directory)
        .unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
    let directory_receipt = record_directory(&File::open(&directory).unwrap(), uid).unwrap();
    let expected = before.with_directory(directory_receipt).unwrap();
    assert_eq!(root.read(), Some(expected.clone()));
    assert!(!expected.directory_missing());
    let text = generated("selected.example").unwrap();
    let path = root.file(&format!("{}/{DROPINS}/{NAME}", ROOTS[0]), &text);
    let receipt = record_file(&File::open(path).unwrap(), uid, &text).unwrap();
    let expected = expected
        .with_file(receipt.clone(), "selected.example")
        .unwrap();
    assert_eq!(root.read(), Some(expected.clone()));
    let mut invalid = receipt;
    invalid.digest = "0".repeat(64);
    assert!(expected.with_file(invalid, "selected.example").is_none());
    root.file(
        "etc/systemd/timesyncd.conf.d/50-external.conf",
        "[Time]\nFallbackNTP=external.example\n",
    );
    assert_ne!(root.read().unwrap(), expected);
}

#[test]
fn private_existing_file_permissions_are_retained_and_receipt_widening_is_refused() {
    let root = Fixture::new();
    let uid = unsafe { libc::geteuid() };
    let path = root.file(
        &format!("{}/{DROPINS}/{NAME}", ROOTS[0]),
        &generated("old.example").unwrap(),
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let before = root.read().unwrap();
    assert_eq!(before.file_policy().unwrap().0, 0o600);
    let text = generated("selected.example").unwrap();
    std::fs::write(&path, &text).unwrap();
    let receipt = record_file(&File::open(&path).unwrap(), uid, &text).unwrap();
    let expected = before.with_file(receipt, "selected.example").unwrap();
    assert_eq!(root.read(), Some(expected));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let widened = record_file(&File::open(&path).unwrap(), uid, &text).unwrap();
    assert!(before.with_file(widened, "selected.example").is_none());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o4755)).unwrap();
    assert!(root.read().is_none());
}

#[test]
fn destination_extended_metadata_is_unavailable_instead_of_silently_dropped() {
    use std::{ffi::CString, os::fd::AsRawFd};
    let set = |file: &File| {
        let name = CString::new("user.agent-manager-fixture").unwrap();
        let value = b"fixture";
        #[cfg(target_os = "linux")]
        let code = unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        };
        #[cfg(target_os = "macos")]
        let code = unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
                0,
            )
        };
        assert_eq!(
            code, 0,
            "Owned extended-attribute fixture could not be created"
        );
        assert!(!plain_metadata(file));
    };
    let root = Fixture::new();
    let text = generated("selected.example").unwrap();
    let path = root.file(&format!("{}/{DROPINS}/{NAME}", ROOTS[0]), &text);
    assert!(root.read().is_some());
    let file = File::open(&path).unwrap();
    set(&file);
    assert!(root.read().is_none());
    assert!(record_file(&file, unsafe { libc::geteuid() }, &text).is_none());
    assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    let root = Fixture::new();
    assert!(root.read().is_some());
    set(&File::open(root.0.join(ROOTS[0])).unwrap());
    assert!(root.read().is_none());
    assert!(!root.0.join(ROOTS[0]).join(DROPINS).exists());
}

#[cfg(target_os = "linux")]
#[test]
fn inode_backup_policy_is_refused_even_when_the_xattr_list_is_empty() {
    use std::os::fd::AsRawFd;
    let root = Fixture::new();
    let text = generated("selected.example").unwrap();
    let path = root.file(&format!("{}/{DROPINS}/{NAME}", ROOTS[0]), &text);
    assert!(root.read().is_some());
    let file = File::open(&path).unwrap();
    let identity = file.metadata().unwrap();
    let mut original: u32 = 0;
    assert_eq!(
        unsafe { libc::ioctl(file.as_raw_fd(), libc::FS_IOC_GETFLAGS, &mut original) },
        0
    );
    let nodump = original | 0x40;
    assert_eq!(
        unsafe { libc::ioctl(file.as_raw_fd(), libc::FS_IOC_SETFLAGS, &nodump) },
        0
    );
    // Positive control for the old admission rule: this real policy has no xattr.
    assert_eq!(
        unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0) },
        0
    );
    assert!(root.read().is_none());
    assert!(record_file(&file, unsafe { libc::geteuid() }, &text).is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    assert_eq!(file.metadata().unwrap().ino(), identity.ino());
    let mut after: u32 = 0;
    assert_eq!(
        unsafe { libc::ioctl(file.as_raw_fd(), libc::FS_IOC_GETFLAGS, &mut after) },
        0
    );
    assert_eq!(after, nodump);
    assert_eq!(
        unsafe { libc::ioctl(file.as_raw_fd(), libc::FS_IOC_SETFLAGS, &original) },
        0
    );
    assert!(root.read().is_some());

    let root = Fixture::new();
    let parent = File::open(root.0.join(ROOTS[0])).unwrap();
    let mut flags: u32 = 0;
    assert!(root.read().is_some());
    assert_eq!(
        unsafe { libc::ioctl(parent.as_raw_fd(), libc::FS_IOC_GETFLAGS, &mut flags) },
        0
    );
    flags |= 0x40;
    assert_eq!(
        unsafe { libc::ioctl(parent.as_raw_fd(), libc::FS_IOC_SETFLAGS, &flags) },
        0
    );
    assert!(root.read().is_none());
    assert!(record_directory(&parent, unsafe { libc::geteuid() }).is_none());
    assert!(!root.0.join(ROOTS[0]).join(DROPINS).exists());

    let root = Fixture::new();
    let parent = root.0.join(ROOTS[0]);
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o2755)).unwrap();
    assert_eq!(std::fs::metadata(&parent).unwrap().mode() & 0o7000, 0o2000);
    assert!(root.read().is_none());
    assert!(!parent.join(DROPINS).exists());
}
