//! Extension dependency operations must not become assistant or account operations.

#[cfg(windows)]
mod common;

use agent_manager_engine::{native::Context, store};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

struct Fixture {
    root: PathBuf,
    ctx: Context,
}

fn link_dir(target: &Path, path: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, path).unwrap();
    #[cfg(windows)]
    junction::create(target, path).unwrap();
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("am-program-scope-{}", store::uuid_v4()));
        #[cfg(windows)]
        common::private_fixture_root(&root);
        let ctx = Context {
            home_dir: root.join("home"),
            data_dir: root.join("data"),
        };
        for dir in [
            ".claude",
            ".codex",
            ".resource-a",
            ".resource-b",
            ".local/bin",
        ] {
            std::fs::create_dir_all(ctx.home_dir.join(dir)).unwrap();
        }
        for dir in [".claude", ".codex", ".resource-a", ".resource-b"] {
            std::fs::write(
                ctx.home_dir.join(dir).join("auth.json"),
                format!("{dir}:unchanged\n"),
            )
            .unwrap();
        }
        let mut policy = store::builtin_policy();
        policy["agents"]["definitions"]["a"] = json!({"bin":"custom-assistant", "upstream_npm":"@example/assistant", "profile":{"base_agent":"claude-code", "config_dir":"~/.resource-a"}});
        // Even a disabled registration with malformed non-path metadata reserves its directory.
        policy["agents"]["definitions"]["b"] =
            json!({"profile":{"base_agent":false, "config_dir":"~/.resource-b"}});
        policy["agents"]["b"] = json!({"enabled":false});
        store::write_policy(&store::open_write(&ctx).unwrap(), &policy).unwrap();
        Self { root, ctx }
    }

    fn cli(&self, command: &str, args: &[&str], entry: &Path) -> Value {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
            .env_clear()
            .env("HOME", &self.ctx.home_dir)
            .env("USERPROFILE", &self.ctx.home_dir)
            .env("AGENT_MANAGER_DATA_DIR", &self.ctx.data_dir)
            .env(
                "AGENT_MANAGER_USER_PATH",
                std::env::join_paths([entry]).unwrap(),
            )
            .arg(command)
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{command}: {:?}", output))
    }

    fn managed(&self, package: &str, bin: &str) -> PathBuf {
        let dir = self
            .ctx
            .data_dir
            .join("programs/npm")
            .join(package.replace('/', "~"));
        let version = dir.join("1.0.0");
        let bins = version.join("node_modules/.bin");
        std::fs::create_dir_all(&bins).unwrap();
        std::fs::write(version.join(".agent-manager-program.json"), "{}").unwrap();
        let name = if cfg!(windows) {
            format!("{bin}.cmd")
        } else {
            bin.into()
        };
        std::fs::write(bins.join(name), "extension-fixture\n").unwrap();
        link_dir(&version, &dir.join("current"));
        dir
    }

    fn identities(&self) -> Vec<Vec<u8>> {
        [".claude", ".codex", ".resource-a", ".resource-b"]
            .iter()
            .map(|dir| std::fs::read(self.ctx.home_dir.join(dir).join("auth.json")).unwrap())
            .collect()
    }

    fn entry_arg(&self, path: &Path) -> String {
        format!(
            "~/{}",
            path.strip_prefix(&self.ctx.home_dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn assistant_packages_and_commands_are_excluded_from_dependency_inventory() {
    let f = Fixture::new();
    let skill = f.ctx.home_dir.join(".claude/skills/dependency");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(skill.join("SKILL.md"), "---\nname: dependency\ndescription: fixture\nmetadata:\n  requires:\n    bins: [claude, codex, custom-assistant]\n---\n```sh\nnpm install -g @anthropic-ai/claude-code\nnpm install -g @openai/codex\n```\n").unwrap();
    std::fs::write(
        f.ctx.home_dir.join(".claude.json"),
        json!({"mcpServers":{
            "assistant-runner":{"command":"npx", "args":["-y", "@openai/codex@1.0.0"]},
            "extension":{"command":"npx", "args":["-y", "extension-fixture@1.0.0"]}
        }})
        .to_string(),
    )
    .unwrap();
    f.managed("@anthropic-ai/claude-code", "claude");
    f.managed("@example/assistant", "custom-assistant");
    let policy = store::effective_policy(&f.ctx).unwrap();
    let identity = f.identities();
    let result = f.cli("programs:status", &[], &f.ctx.home_dir.join(".local/bin"));
    assert_eq!(result["ok"], true, "{result}");
    let data = result["data"].to_string();
    for forbidden in [
        "bin:claude",
        "bin:codex",
        "bin:custom-assistant",
        "npm:@openai/codex",
        "npm:@anthropic-ai/claude-code",
        "npm:@example/assistant",
    ] {
        assert!(
            !data.contains(forbidden),
            "assistant exposed as dependency: {forbidden}: {result}"
        );
    }
    assert!(
        data.contains("npm:extension-fixture"),
        "ordinary MCP dependency lost: {result}"
    );
    assert_eq!(policy, store::effective_policy(&f.ctx).unwrap());
    assert_eq!(identity, f.identities());
}

#[test]
fn direct_dependency_commands_cannot_manage_assistants() {
    let f = Fixture::new();
    let entry = f.ctx.home_dir.join(".local/bin");
    let policy = store::effective_policy(&f.ctx).unwrap();
    let identity = f.identities();
    for (package, bin) in [
        ("@anthropic-ai/claude-code", "fixture-bin"),
        ("@openai/codex", "fixture-bin"),
        ("@example/assistant", "fixture-bin"),
        ("unusual-assistant-package", "codex"),
    ] {
        let dir = f.managed(package, bin);
        let program = format!("npm:{package}");
        for command in ["programs:preview", "programs:apply"] {
            for action in [
                "link", "unlink", "remove", "switch", "upgrade", "adopt", "restore", "unadopt",
            ] {
                let result = f.cli(
                    command,
                    &[
                        "--program",
                        &program,
                        "--action",
                        action,
                        "--version",
                        "1.0.0",
                        "--entry-dir",
                        &f.entry_arg(&entry),
                    ],
                    &entry,
                );
                assert_eq!(result["ok"], false, "{command} {action}: {result}");
                assert!(
                    result["errors"].to_string().contains("助手"),
                    "not refused by scope guard: {result}"
                );
                assert!(dir.join("current").exists());
            }
        }
        if bin == "fixture-bin" {
            let result = f.cli(
                "programs:declare",
                &["--program", &program, "--allow-scripts", "true"],
                &entry,
            );
            assert_eq!(result["ok"], false, "{result}");
        }
    }
    for bin in ["claude", "codex", "custom-assistant"] {
        for args in [
            vec!["--skill", "dependency", "--bins", &format!("[\"{bin}\"]")],
            vec!["--bin", bin, "--program", "npm:extension-fixture"],
        ] {
            assert_eq!(f.cli("programs:declare", &args, &entry)["ok"], false);
        }
    }
    assert_eq!(policy, store::effective_policy(&f.ctx).unwrap());
    assert_eq!(identity, f.identities());
    assert!(!entry.join("fixture-bin").exists());
}

#[test]
fn dependency_entry_directories_cannot_overlap_accounts() {
    let f = Fixture::new();
    f.managed("extension-fixture", "fixture-bin");
    let alias = f.ctx.home_dir.join("entry-alias");
    link_dir(&f.ctx.home_dir.join(".resource-a"), &alias);
    std::fs::create_dir_all(f.ctx.home_dir.join(".resource-a/bin")).unwrap();
    let identity = f.identities();
    let policy = store::effective_policy(&f.ctx).unwrap();
    for relative in [
        ".claude",
        ".codex",
        ".resource-a",
        ".resource-b",
        ".resource-a/bin",
        "entry-alias",
    ] {
        let entry = f.ctx.home_dir.join(relative);
        for command in ["programs:preview", "programs:apply"] {
            let result = f.cli(
                command,
                &[
                    "--program",
                    "npm:extension-fixture",
                    "--action",
                    "link",
                    "--entry-dir",
                    &f.entry_arg(&entry),
                ],
                &entry,
            );
            assert_eq!(
                result["ok"], false,
                "unsafe entry accepted: {relative}: {result}"
            );
            assert!(result["errors"].to_string().contains("账号"), "{result}");
        }
        let name = if cfg!(windows) {
            "fixture-bin.cmd"
        } else {
            "fixture-bin"
        };
        assert!(!entry.join(name).exists());
    }
    assert_eq!(identity, f.identities());
    assert_eq!(policy, store::effective_policy(&f.ctx).unwrap());
}

#[test]
fn ordinary_extension_entry_can_be_linked_and_unlinked() {
    let f = Fixture::new();
    f.managed("extension-fixture", "fixture-bin");
    let entry = f.ctx.home_dir.join(".local/bin");
    let identity = f.identities();
    for action in ["link", "unlink"] {
        let result = f.cli(
            "programs:apply",
            &[
                "--program",
                "npm:extension-fixture",
                "--action",
                action,
                "--entry-dir",
                &f.entry_arg(&entry),
            ],
            &entry,
        );
        assert_eq!(result["ok"], true, "{result}");
        let name = if cfg!(windows) {
            "fixture-bin.cmd"
        } else {
            "fixture-bin"
        };
        assert_eq!(entry.join(name).exists(), action == "link");
    }
    assert_eq!(identity, f.identities());
}

#[test]
fn historical_dependency_links_inside_accounts_are_not_cleaned_up() {
    let f = Fixture::new();
    let managed = f.managed("extension-fixture", "fixture-bin");
    let target = managed
        .join("current/node_modules/.bin")
        .join(if cfg!(windows) {
            "fixture-bin.cmd"
        } else {
            "fixture-bin"
        });
    let path = f.ctx.home_dir.join(".resource-b").join(if cfg!(windows) {
        "fixture-bin.cmd"
    } else {
        "fixture-bin"
    });
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &path).unwrap();
    #[cfg(windows)]
    agent_manager_engine::platform::write_cmd_shim(&target, &path).unwrap();
    let mut policy = store::effective_policy(&f.ctx).unwrap();
    let shim = f.ctx.data_dir.join("programs/bin").join(if cfg!(windows) {
        "fixture-bin.cmd"
    } else {
        "fixture-bin"
    });
    policy["programs"]["links"] = json!({ f.entry_arg(&path): shim.to_string_lossy() });
    store::write_policy(&store::open_write(&f.ctx).unwrap(), &policy).unwrap();
    let identity = f.identities();
    for command in ["programs:preview", "programs:apply"] {
        let result = f.cli(
            command,
            &["--program", "npm:extension-fixture", "--action", "remove"],
            &f.ctx.home_dir.join(".local/bin"),
        );
        assert_eq!(result["ok"], false, "{result}");
        assert!(result["errors"].to_string().contains("账号"), "{result}");
        assert!(path.exists());
        assert!(managed.join("current").exists());
    }
    assert_eq!(identity, f.identities());
    assert_eq!(policy, store::effective_policy(&f.ctx).unwrap());
}

#[test]
fn dependency_backup_cannot_restore_into_an_account_directory() {
    let f = Fixture::new();
    let backup = f.ctx.data_dir.join("programs/backups/fixture-backup");
    std::fs::create_dir_all(backup.join("package")).unwrap();
    std::fs::write(backup.join("package/fixture"), "preserve-backup\n").unwrap();
    std::fs::write(backup.join("backup.json"), json!({"program":"npm:extension-fixture", "package_dir":f.ctx.home_dir.join(".resource-a"), "links":[]}).to_string()).unwrap();
    let identity = f.identities();
    for command in ["programs:preview", "programs:apply"] {
        let result = f.cli(
            command,
            &[
                "--program",
                "npm:extension-fixture",
                "--action",
                "restore",
                "--backup",
                "fixture-backup",
            ],
            &f.ctx.home_dir.join(".local/bin"),
        );
        assert_eq!(result["ok"], false, "{result}");
        assert!(result["errors"].to_string().contains("账号"), "{result}");
        assert_eq!(identity, f.identities());
        assert_eq!(
            std::fs::read_to_string(backup.join("package/fixture")).unwrap(),
            "preserve-backup\n"
        );
    }
}

#[test]
fn dependency_internal_directory_alias_cannot_redirect_writes_into_an_account() {
    let f = Fixture::new();
    f.managed("extension-fixture", "fixture-bin");
    let alias = f.ctx.data_dir.join("programs/bin");
    link_dir(&f.ctx.home_dir.join(".resource-a"), &alias);
    let identity = f.identities();
    let policy = store::effective_policy(&f.ctx).unwrap();
    let entry = f.ctx.home_dir.join(".local/bin");
    for command in ["programs:preview", "programs:apply"] {
        let result = f.cli(
            command,
            &[
                "--program",
                "npm:extension-fixture",
                "--action",
                "link",
                "--entry-dir",
                &f.entry_arg(&entry),
            ],
            &entry,
        );
        assert_eq!(result["ok"], false, "{result}");
        assert!(result["errors"].to_string().contains("账号"), "{result}");
        assert_eq!(identity, f.identities());
        assert_eq!(policy, store::effective_policy(&f.ctx).unwrap());
    }
}

#[test]
fn dependency_backup_pruning_cannot_follow_an_account_directory_alias() {
    let f = Fixture::new();
    f.managed("extension-fixture", "fixture-bin");
    let account = f.ctx.home_dir.join(".resource-a");
    for index in 0..4 {
        let backup = account.join(format!("backup-{index}"));
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(backup.join("backup.json"), json!({"program":"npm:extension-fixture", "created_at":format!("2026-01-0{}", index+1)}).to_string()).unwrap();
    }
    link_dir(&account, &f.ctx.data_dir.join("programs/backups"));
    let entry = f.ctx.home_dir.join(".local/bin");
    let identity = f.identities();
    let result = f.cli(
        "programs:apply",
        &[
            "--program",
            "npm:extension-fixture",
            "--action",
            "link",
            "--entry-dir",
            &f.entry_arg(&entry),
        ],
        &entry,
    );
    assert_eq!(result["ok"], true, "ordinary operation failed: {result}");
    for index in 0..4 {
        assert!(
            account
                .join(format!("backup-{index}/backup.json"))
                .is_file(),
            "account file removed by optional cleanup"
        );
    }
    assert_eq!(identity, f.identities());
    let status = f.cli("programs:status", &[], &entry);
    assert_eq!(status["ok"], true, "{status}");
    assert_eq!(
        status["data"]["backups"],
        json!([]),
        "account files exposed as dependency backups: {status}"
    );
}
