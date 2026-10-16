#![cfg(windows)]

mod common;
#[path = "common/skill_repo.rs"]
mod skill_repo;

use serde_json::Value;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "am-materialized-cli-{}",
            agent_manager_engine::store::uuid_v4()
        ));
        common::private_fixture_root(&path);
        for directory in ["home", "tmp"] {
            std::fs::create_dir(path.join(directory)).unwrap();
        }
        Self(path)
    }

    fn run(&self, args: &[&str], expected: bool) -> Value {
        let stdout = self.0.join("stdout");
        let stderr = self.0.join("stderr");
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"));
        command.env_clear();
        for name in ["SystemRoot", "WINDIR", "COMSPEC", "PATHEXT"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .env(
                "PATH",
                PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32"),
            )
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("TEMP", self.0.join("tmp"))
            .env("TMP", self.0.join("tmp"))
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
                "Owned skill CLI exceeded its deadline"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let bytes = std::fs::read(stdout).unwrap();
        assert!(bytes.len() < 1024 * 1024);
        let result: Value = serde_json::from_slice(&bytes).unwrap();
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

    fn version(&self, agent: &str) -> String {
        self.cell(agent)["version"].as_str().unwrap().into()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
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

fn assert_ordinary(path: &Path) {
    let metadata = std::fs::symlink_metadata(path).unwrap();
    assert!(metadata.is_file() && !metadata.file_type().is_symlink());
    assert_eq!(
        metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT,
        0
    );
    assert_eq!(std::fs::read(path).unwrap(), b"default stream payload");
}

#[test]
fn native_cli_materialized_skill_lifecycle_uses_ordinary_files_and_offline_snapshots() {
    let fixture = Fixture::new();
    let repo = fixture.0.join("data/repositories/upstream");
    let source = repo.join("skills/alpha");
    std::fs::create_dir_all(source.join("nested/empty")).unwrap();
    std::fs::write(source.join("SKILL.md"), b"# Fixture\n").unwrap();
    std::fs::write(source.join("z-target"), b"default stream payload").unwrap();
    std::fs::write(
        source.join("z-target:materialization-fixture"),
        b"unhashed stream",
    )
    .unwrap();
    for (path, raw) in [
        ("alias", r".\z-target"),
        ("nested/alias", r".\empty\..\..\z-target"),
    ] {
        std::os::windows::fs::symlink_file(raw, source.join(path)).unwrap();
    }
    skill_repo::index(&repo);
    fixture.run(
        &[
            "policy:set",
            "--layer",
            "local",
            "--key",
            "skills.repositories.upstream",
            "--value",
            r#"{"url":"https://example.com/upstream.git","skill_paths":["skills"]}"#,
        ],
        true,
    );
    fixture.action("add", "codex", true);
    let version = fixture.version("codex");
    let snapshot = fixture.0.join("data/skill-versions").join(&version);
    let manifest_bytes = std::fs::read(snapshot.join("snapshot.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["schema"], 3);
    assert_eq!(
        manifest["materialized_file_links"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let content = snapshot.join("content");
    for path in ["alias", "nested/alias", "z-target"] {
        assert_ordinary(&content.join(path));
        assert!(std::fs::read(content.join(format!("{path}:materialization-fixture"))).is_err());
    }
    assert!(content.join("nested/empty").is_dir());
    fixture.action("add", "claude-code", true);
    assert_eq!(fixture.version("claude-code"), version);
    assert_eq!(
        std::fs::read(snapshot.join("snapshot.json")).unwrap(),
        manifest_bytes
    );
    fixture.action("disable", "codex", true);
    let other_link = fixture.0.join("home/.claude/skills/alpha");
    let other_target = std::fs::canonicalize(&other_link).unwrap();
    let other_cell = fixture.cell("claude-code");
    assert!(std::fs::symlink_metadata(fixture.0.join("home/.codex/skills/alpha")).is_err());
    std::fs::write(content.join("alias"), b"tampered").unwrap();
    fixture.action("enable", "codex", false);
    assert_eq!(fixture.version("codex"), version);
    assert_eq!(fixture.cell("codex")["enabled"], false);
    assert_eq!(fixture.cell("claude-code"), other_cell);
    assert_eq!(std::fs::canonicalize(&other_link).unwrap(), other_target);
    assert!(std::fs::symlink_metadata(fixture.0.join("home/.codex/skills/alpha")).is_err());
    std::fs::write(content.join("alias"), b"default stream payload").unwrap();
    fixture.action("enable", "codex", true);
    assert_ordinary(&fixture.0.join("home/.codex/skills/alpha/alias"));
    std::fs::rename(&repo, fixture.0.join("offline-source")).unwrap();
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
    for path in ["alias", "nested/alias", "z-target"] {
        assert_ordinary(&destination.join("alpha").join(path));
    }
    assert!(!destination.join("alpha/snapshot.json").exists());
    std::fs::write(destination.join("alpha/alias"), b"editable personal copy").unwrap();
    assert_ordinary(&content.join("alias"));
    assert_eq!(fixture.version("claude-code"), version);
}
