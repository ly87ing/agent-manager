//! Actual CLI and Git checkout, without Git on the CLI process's PATH.
#[cfg(windows)]
mod common;
#[path = "common/skill_repo.rs"]
mod skill_repo;

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Fixture(PathBuf);

impl Fixture {
    fn new(format: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "am-git-links-cli-{}",
            agent_manager_engine::store::uuid_v4()
        ));
        #[cfg(windows)]
        common::private_fixture_root(&root);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::create_dir(&root).unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        for directory in [
            "home",
            "tmp",
            "empty-bin",
            "data/repositories/upstream/skills/alpha",
        ] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        let fixture = Self(root);
        let source = fixture.source();
        std::fs::write(source.join("SKILL.md"), b"# Fixture\n").unwrap();
        std::fs::write(source.join("z-target"), b"ordinary target bytes\n").unwrap();
        std::fs::write(source.join("alias"), b"./z-target").unwrap();
        fixture.git(&[
            "init",
            "--quiet",
            &format!("--object-format={format}"),
            "--template=",
        ]);
        fixture.git(&["config", "core.symlinks", "false"]);
        fixture.git(&["add", "-A", "--", "."]);
        let id = fixture.git(&["hash-object", "-w", "--", "skills/alpha/alias"]);
        fixture.git(&[
            "update-index",
            "--cacheinfo",
            &format!("120000,{},skills/alpha/alias", id.trim()),
        ]);
        fixture.git(&["checkout-index", "--force", "--", "skills/alpha/alias"]);
        assert!(fixture
            .git(&["ls-files", "--stage", "--", "skills/alpha/alias"])
            .starts_with("120000 "));
        assert!(std::fs::symlink_metadata(source.join("alias"))
            .unwrap()
            .is_file());
        assert_eq!(std::fs::read(source.join("alias")).unwrap(), b"./z-target");
        fixture.run(
            &[
                "policy:set",
                "--layer",
                "local",
                "--key",
                "skills.repositories.upstream",
                "--value",
                r#"{"url":"https://example.invalid/YOUR_REPOSITORY.git","skill_paths":["skills"]}"#,
            ],
            true,
        );
        fixture
    }

    fn repo(&self) -> PathBuf {
        self.0.join("data/repositories/upstream")
    }
    fn source(&self) -> PathBuf {
        self.repo().join("skills/alpha")
    }
    fn git(&self, args: &[&str]) -> String {
        skill_repo::git(&self.repo(), args)
    }

    fn run(&self, args: &[&str], expected: bool) -> Value {
        let stdout = self.0.join("cli-stdout");
        let stderr = self.0.join("cli-stderr");
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"));
        command.env_clear();
        for name in ["SystemRoot", "WINDIR", "COMSPEC", "PATHEXT"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .env("PATH", self.0.join("empty-bin"))
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("APPDATA", self.0.join("home/appdata"))
            .env("LOCALAPPDATA", self.0.join("home/localappdata"))
            .env("TEMP", self.0.join("tmp"))
            .env("TMP", self.0.join("tmp"))
            .env("TMPDIR", self.0.join("tmp"))
            .env("AGENT_MANAGER_DATA_DIR", self.0.join("data"))
            .args(args)
            .arg("--json")
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&stdout).unwrap())
            .stderr(std::fs::File::create(&stderr).unwrap());
        let mut child = OwnedChild(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(60);
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "Owned Git-skill CLI exceeded its deadline"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(std::fs::metadata(&stdout).unwrap().len() < 1024 * 1024);
        let result: Value = serde_json::from_slice(&std::fs::read(stdout).unwrap()).unwrap();
        assert_eq!(status.success(), expected, "{result}");
        assert_eq!(result["ok"], expected, "{result}");
        result
    }

    fn action(&self, action: &str, agent: &str, expected: bool) {
        self.run(
            &[
                "skills:apply",
                "--only",
                "skill:upstream/alpha",
                "--action",
                action,
                "--agent",
                agent,
            ],
            expected,
        );
    }

    fn cell(&self, agent: &str) -> Value {
        let status = self.run(&["skills:status"], true);
        let row = status["data"]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == "skill:upstream/alpha")
            .unwrap();
        row["observed"]["agents"][agent].clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("Retaining failed CLI fixture: {}", self.0.display());
        } else {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn git_metadata(root: &Path) -> BTreeMap<PathBuf, (String, std::time::SystemTime)> {
    let mut result = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for child in std::fs::read_dir(directory).unwrap() {
            let path = child.unwrap().path();
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            assert!(!metadata.file_type().is_symlink());
            if metadata.is_dir() {
                result.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    ("directory".into(), metadata.modified().unwrap()),
                );
                stack.push(path);
            } else {
                assert!(metadata.len() < 1024 * 1024);
                result.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    (
                        format!("{:x}", Sha256::digest(std::fs::read(path).unwrap())),
                        metadata.modified().unwrap(),
                    ),
                );
            }
        }
    }
    result
}

fn ordinary_payload(path: &Path) {
    let metadata = std::fs::symlink_metadata(path).unwrap();
    assert!(metadata.is_file() && !metadata.file_type().is_symlink());
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        assert_eq!(
            metadata.file_attributes()
                & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT,
            0
        );
    }
    assert_eq!(std::fs::read(path).unwrap(), b"ordinary target bytes\n");
}

fn tamper_snapshot(path: &Path, bytes: &[u8]) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444)).unwrap();
    }
}

#[test]
fn native_cli_git_plaintext_links_are_verified_materialized_and_reusable_offline() {
    for format in ["sha1", "sha256"] {
        let fixture = Fixture::new(format);
        let metadata = git_metadata(&fixture.repo().join(".git"));
        assert!(fixture.cell("codex")["available_version"].is_string());
        fixture.action("add", "codex", true);
        let version = fixture.cell("codex")["version"]
            .as_str()
            .unwrap()
            .to_string();
        let snapshot = fixture.0.join("data/skill-versions").join(&version);
        let receipt = std::fs::read(snapshot.join("snapshot.json")).unwrap();
        let manifest: Value = serde_json::from_slice(&receipt).unwrap();
        assert_eq!(manifest["schema"], 3);
        assert_eq!(
            manifest["materialized_file_links"][0]["raw_target"],
            "./z-target"
        );
        ordinary_payload(&snapshot.join("content/alias"));
        #[cfg(unix)]
        assert!(std::fs::metadata(snapshot.join("content/alias"))
            .unwrap()
            .permissions()
            .readonly());
        assert_eq!(git_metadata(&fixture.repo().join(".git")), metadata);

        let index = fixture.repo().join(".git/index");
        let saved = fixture.0.join("saved-index");
        std::fs::rename(&index, &saved).unwrap();
        let cell = fixture.cell("codex");
        assert!(cell["available_version"].is_null());
        assert!(!cell["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action == "update"));
        fixture.action("update", "codex", false);
        fixture.action("add", "claude-code", false);
        fixture.action("disable", "codex", true);
        fixture.action("enable", "codex", true);
        assert_eq!(fixture.cell("codex")["version"], version);
        std::fs::rename(&saved, &index).unwrap();
        fixture.action("add", "claude-code", true);
        assert_eq!(fixture.cell("claude-code")["version"], version);
        assert_eq!(
            std::fs::read(snapshot.join("snapshot.json")).unwrap(),
            receipt
        );
        assert_eq!(git_metadata(&fixture.repo().join(".git")), metadata);

        fixture.action("disable", "codex", true);
        let other = fixture.cell("claude-code");
        tamper_snapshot(&snapshot.join("content/alias"), b"tampered");
        fixture.action("enable", "codex", false);
        assert_eq!(fixture.cell("codex")["enabled"], false);
        assert_eq!(fixture.cell("claude-code"), other);
        tamper_snapshot(&snapshot.join("content/alias"), b"ordinary target bytes\n");
        fixture.action("enable", "codex", true);

        #[cfg(unix)]
        {
            std::fs::remove_file(fixture.source().join("alias")).unwrap();
            std::os::unix::fs::symlink("./z-target", fixture.source().join("alias")).unwrap();
            assert_eq!(fixture.cell("codex")["available_version"], version);
            fixture.action("update", "codex", true);
            assert_eq!(fixture.cell("codex")["version"], version);
        }
        fixture.git(&["status", "--porcelain"]);
        assert_eq!(fixture.cell("codex")["available_version"], version);
        std::fs::rename(fixture.repo(), fixture.0.join("offline-source")).unwrap();
        let destination = fixture.0.join("personal");
        fixture.run(
            &[
                "skills:fork",
                "--only",
                "skill:upstream/alpha",
                "--dest",
                destination.to_str().unwrap(),
            ],
            true,
        );
        ordinary_payload(&destination.join("alpha/alias"));
        assert!(!destination.join("alpha/snapshot.json").exists());
        std::fs::write(destination.join("alpha/alias"), b"editable").unwrap();
        ordinary_payload(&snapshot.join("content/alias"));
    }
}

#[test]
fn plaintext_directory_links_and_nested_gitlinks_do_not_publish_raw_text_snapshots() {
    let fixture = Fixture::new("sha1");
    std::fs::create_dir(fixture.source().join("directory")).unwrap();
    std::fs::write(fixture.source().join("directory/payload"), b"fixture").unwrap();
    std::fs::write(fixture.source().join("alias"), b"directory").unwrap();
    let metadata = git_metadata(&fixture.repo().join(".git"));
    fixture.action("add", "codex", false);
    assert!(!fixture.0.join("data/skill-versions").exists());
    assert_eq!(git_metadata(&fixture.repo().join(".git")), metadata);

    std::fs::write(fixture.source().join("alias"), b"./z-target").unwrap();
    let tools = fixture.source().join("tools");
    std::fs::create_dir(&tools).unwrap();
    std::fs::write(tools.join("alias"), b"./unindexed-target").unwrap();
    let id = fixture.git(&["hash-object", "-w", "--", "skills/alpha/alias"]);
    fixture.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{},skills/alpha/tools", id.trim()),
    ]);
    let metadata = git_metadata(&fixture.repo().join(".git"));
    assert!(fixture.cell("codex")["available_version"].is_null());
    fixture.action("add", "codex", false);
    assert!(!fixture.0.join("data/skill-versions").exists());
    assert_eq!(
        std::fs::read(tools.join("alias")).unwrap(),
        b"./unindexed-target"
    );
    assert_eq!(git_metadata(&fixture.repo().join(".git")), metadata);
}

#[test]
fn directory_link_skill_roots_and_changed_link_spelling_are_unknown() {
    let fixture = Fixture::new("sha1");
    let outside = fixture.0.join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("SKILL.md"), b"# Outside\n").unwrap();
    std::fs::write(outside.join("alias"), b"./target").unwrap();
    let aliased = fixture.repo().join("skills/aliased");
    agent_manager_engine::platform::link_dir(&outside, &aliased).unwrap();
    fixture.run(
        &[
            "skills:apply",
            "--only",
            "skill:upstream/aliased",
            "--action",
            "add",
            "--agent",
            "codex",
        ],
        false,
    );
    assert!(!fixture.0.join("data/skill-versions").exists());
    assert!(!fixture.0.join("home/.codex/skills/aliased").exists());
    agent_manager_engine::platform::remove_link(&aliased).unwrap();
    std::fs::rename(
        fixture.source().join("alias"),
        fixture.source().join("ALIAS"),
    )
    .unwrap();
    assert!(fixture.cell("codex")["available_version"].is_null());
    fixture.action("add", "codex", false);
    assert!(!fixture.0.join("data/skill-versions").exists());
}

#[test]
fn filesystem_aliases_in_other_index_prefixes_cannot_supply_file_kinds() {
    for kind in ["120000", "160000"] {
        let fixture = Fixture::new("sha1");
        if std::fs::symlink_metadata(fixture.repo().join("skills/ALPHA/SKILL.md")).is_err() {
            eprintln!("Filesystem alias challenge requires a case-insensitive filesystem");
            continue;
        }
        let id = fixture.git(&["hash-object", "-w", "--", "skills/alpha/alias"]);
        fixture.git(&[
            "update-index",
            "--cacheinfo",
            &format!("100644,{},skills/alpha/alias", id.trim()),
        ]);
        if kind == "120000" {
            let marker = fixture.git(&["hash-object", "-w", "--", "skills/alpha/SKILL.md"]);
            fixture.git(&[
                "-c",
                "core.ignorecase=false",
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("100644,{},skills/ALPHA/SKILL.md", marker.trim()),
            ]);
            fixture.git(&[
                "-c",
                "core.ignorecase=false",
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("120000,{},skills/ALPHA/alias", id.trim()),
            ]);
        } else {
            fixture.git(&[
                "-c",
                "core.ignorecase=false",
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{},skills/ALPHA", id.trim()),
            ]);
        }
        let metadata = git_metadata(&fixture.repo().join(".git"));
        assert!(fixture.cell("codex")["available_version"].is_null());
        fixture.action("add", "codex", false);
        assert!(!fixture.0.join("data/skill-versions").exists());
        assert_eq!(git_metadata(&fixture.repo().join(".git")), metadata);
        eprintln!("Case-insensitive filesystem alias challenge exercised for {kind}");
    }
}
