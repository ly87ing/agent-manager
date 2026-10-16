//! Real Git metadata for explicitly prepared skill-cache fixtures.
use agent_manager_engine::platform::Canon;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

pub fn git(repo: &Path, args: &[&str]) -> String {
    let temporary = std::env::temp_dir().canon().unwrap();
    let repo = repo.canon().unwrap();
    let relative = repo.strip_prefix(&temporary).unwrap();
    let first = relative.components().next().unwrap().as_os_str();
    assert!(first.to_string_lossy().starts_with("am-"));
    let root = temporary.join(first);
    let empty = root.join("empty-fixture-git-config");
    if !empty.exists() {
        std::fs::write(&empty, b"").unwrap();
    }
    let mut command = agent_manager_engine::platform::git_command();
    let stdout = root.join("fixture-git-stdout");
    let stderr = root.join("fixture-git-stderr");
    command
        .current_dir(&repo)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", &empty)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&stdout).unwrap())
        .stderr(std::fs::File::create(&stderr).unwrap());
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Owned Git fixture command exceeded its deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(std::fs::metadata(&stdout).unwrap().len() <= 64 * 1024);
    assert!(std::fs::metadata(&stderr).unwrap().len() <= 64 * 1024);
    assert!(
        status.success(),
        "{}",
        std::fs::read_to_string(stderr).unwrap()
    );
    std::fs::read_to_string(stdout).unwrap()
}

#[allow(dead_code)] // Some fixtures prepare a different object format directly.
pub fn index(repo: &Path) {
    git(
        repo,
        &["init", "--quiet", "--object-format=sha1", "--template="],
    );
    git(repo, &["-c", "core.symlinks=true", "add", "-A", "--", "."]);
}
