use super::*;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("am-ntp-publish-{}", crate::store::uuid_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        let root = Self(std::fs::canonicalize(path).unwrap());
        std::fs::create_dir_all(root.0.join("etc/systemd")).unwrap();
        root
    }
    fn state(&self) -> config::State {
        config::read_at(&self.0, unsafe { libc::geteuid() }).unwrap()
    }
    fn lock(&self, state: &config::State) -> Option<HostLock> {
        HostLock::at(&self.0, unsafe { libc::geteuid() }, state)
    }
    fn target(&self) -> PathBuf {
        self.0.join(DIRECTORY).join(config::NAME)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn publication_reads_back_exact_owned_changes_and_noop_preserves_the_snapshot() {
    let root = Fixture::new();
    let before = root.state();
    let lock = root.lock(&before).unwrap();
    assert!(
        root.lock(&before).is_none(),
        "A second helper acquired the host lock"
    );
    let result = lock.publish(&before, "Selected.Example.", |_| true);
    assert_eq!(result.status, "saved");
    assert!(result.durable);
    assert_eq!(
        std::fs::read_to_string(root.target()).unwrap(),
        config::generated("selected.example").unwrap()
    );
    assert_eq!(
        std::fs::metadata(root.target()).unwrap().mode() & 0o777,
        0o644
    );
    assert_eq!(std::fs::metadata(root.target()).unwrap().nlink(), 1);
    let expected = before
        .with_directory(result.directory.unwrap())
        .unwrap()
        .with_file(result.file.unwrap(), "selected.example")
        .unwrap();
    assert_eq!(root.state(), expected);
    let noop = lock.publish(&expected, "selected.example", |_| true);
    assert_eq!(noop.status, "unchanged");
    assert!(noop.directory.is_none() && noop.file.is_none());
    assert_eq!(root.state(), expected);
    assert_eq!(
        std::fs::read_dir(root.target().parent().unwrap())
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn stale_or_cancelled_publication_never_creates_the_setting_directory() {
    let root = Fixture::new();
    let before = root.state();
    let lock = root.lock(&before).unwrap();
    let cancelled = lock.publish(&before, "selected.example", |_| false);
    assert_eq!(cancelled.status, "stale");
    assert!(!root.target().parent().unwrap().exists());
    let main = root.0.join("etc/systemd/timesyncd.conf");
    std::fs::write(&main, "[Time]\nNTP=external.example\n").unwrap();
    let external = std::fs::read(&main).unwrap();
    let stale = lock.publish(&before, "selected.example", |_| true);
    assert_eq!(stale.status, "stale");
    assert!(!root.target().parent().unwrap().exists());
    assert_eq!(std::fs::read(main).unwrap(), external);
}

#[test]
fn creation_cannot_overwrite_a_target_inserted_after_the_final_read() {
    let root = Fixture::new();
    let before = root.state();
    let lock = root.lock(&before).unwrap();
    let calls = std::cell::Cell::new(0);
    let result = lock.publish(&before, "selected.example", |_| {
        calls.set(calls.get() + 1);
        if calls.get() == 3 {
            std::fs::write(root.target(), "external contents").unwrap();
        }
        true
    });
    assert_eq!(
        calls.get(),
        3,
        "The race fixture did not reach pre-publication"
    );
    assert_eq!(result.status, "failed");
    assert!(result.file.is_none());
    assert_eq!(
        std::fs::read_to_string(root.target()).unwrap(),
        "external contents"
    );
    assert_eq!(
        std::fs::read_dir(root.target().parent().unwrap())
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn replacement_preserves_private_mode_and_does_not_leave_staging_files() {
    let root = Fixture::new();
    std::fs::create_dir(root.target().parent().unwrap()).unwrap();
    std::fs::write(root.target(), config::generated("old.example").unwrap()).unwrap();
    std::fs::set_permissions(root.target(), std::fs::Permissions::from_mode(0o600)).unwrap();
    let before = root.state();
    let group = std::fs::metadata(root.target()).unwrap().gid();
    let lock = root.lock(&before).unwrap();
    let result = lock.publish(&before, "selected.example", |_| true);
    assert_eq!(result.status, "saved");
    assert!(result.directory.is_none());
    assert!(result.durable);
    let meta = std::fs::metadata(root.target()).unwrap();
    assert_eq!(meta.mode() & 0o777, 0o600);
    assert_eq!(meta.gid(), group);
    assert_eq!(
        root.state(),
        before
            .with_file(result.file.unwrap(), "selected.example")
            .unwrap()
    );
    assert_eq!(
        std::fs::read_dir(root.target().parent().unwrap())
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn staging_cleanup_preserves_a_replaced_directory_entry() {
    let root = Fixture::new();
    std::fs::create_dir(root.target().parent().unwrap()).unwrap();
    let directory = File::open(root.target().parent().unwrap()).unwrap();
    let stage = Stage::create(&directory, unsafe { libc::geteuid() }).unwrap();
    let name = root.target().parent().unwrap().join(&stage.name);
    let original = name.with_extension("owned");
    std::fs::rename(&name, &original).unwrap();
    std::fs::write(&name, "external contents").unwrap();
    drop(stage);
    assert_eq!(std::fs::read_to_string(&name).unwrap(), "external contents");
    assert!(original.is_file());
}
