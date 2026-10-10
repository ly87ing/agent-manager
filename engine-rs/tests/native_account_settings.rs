//! Main runtime configuration cannot select or alias another account's files.

use agent_manager_engine::{execute, store, Context, Envelope, Request};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const CLAUDE: &[u8] = b"{\"model\":\"original\",\"env\":{\"MY_OWN\":\"keep\"}}\n";
const CODEX: &[u8] = b"model = \"original\"\nuser_instructions = \"keep\"\n";

struct Fixture {
    root: PathBuf,
    ctx: Context,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("am-account-settings-{}", store::uuid_v4()));
        fs::create_dir_all(&root).unwrap();
        let root = agent_manager_engine::platform::Canon::canon(root.as_path()).unwrap();
        let ctx = Context {
            home_dir: root.join("home"),
            data_dir: root.join("data"),
        };
        for dir in [".claude", ".codex", ".account-a", ".account-b"] {
            fs::create_dir_all(ctx.home_dir.join(dir)).unwrap();
            fs::write(ctx.home_dir.join(dir).join("settings.json"), CLAUDE).unwrap();
            fs::write(ctx.home_dir.join(dir).join("config.toml"), CODEX).unwrap();
            fs::write(
                ctx.home_dir.join(dir).join("auth.json"),
                b"{\"dummy\":true}\n",
            )
            .unwrap();
        }
        fs::write(ctx.home_dir.join(".claude.json"), b"{\"dummy\":true}\n").unwrap();
        // The dispatcher's existing global lock is separate from account state.
        fs::create_dir_all(&ctx.data_dir).unwrap();
        fs::write(ctx.data_dir.join("agent-manager.apply.lock"), b"").unwrap();
        Self { root, ctx }
    }

    fn legacy(&self, change: impl FnOnce(&mut Value)) {
        let mut policy = store::effective_policy(&self.ctx).unwrap();
        change(&mut policy);
        store::write_policy(&store::open_write(&self.ctx).unwrap(), &policy).unwrap();
    }

    fn profiles(&self) {
        self.legacy(|policy| {
            for id in ["account-a", "account-b"] {
                policy["agents"]["definitions"][id] = json!({
                    "bin":"claude", "profile":{"base_agent":"claude-code", "config_dir":format!("~/.{id}")}
                });
            }
        });
    }

    fn unchanged(&self) {
        for dir in [".claude", ".codex", ".account-a", ".account-b"] {
            assert_eq!(
                fs::read(self.ctx.home_dir.join(dir).join("settings.json")).unwrap(),
                CLAUDE
            );
            assert_eq!(
                fs::read(self.ctx.home_dir.join(dir).join("config.toml")).unwrap(),
                CODEX
            );
            assert_eq!(
                fs::read(self.ctx.home_dir.join(dir).join("auth.json")).unwrap(),
                b"{\"dummy\":true}\n"
            );
        }
        assert_eq!(
            fs::read(self.ctx.home_dir.join(".claude.json")).unwrap(),
            b"{\"dummy\":true}\n"
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn request(command: &str, args: &[&str]) -> Request {
    Request {
        command: command.into(),
        args: args.iter().map(|arg| (*arg).into()).collect(),
        run_id: "account-settings-test".into(),
    }
}

fn current(ctx: &Context, id: &str) -> Value {
    let result = execute(ctx, &request("agents:status", &["--offline"]));
    assert!(result.ok, "{:?}", result.errors);
    result.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == format!("agents:{id}"))
        .unwrap()["observed"]["current_config"]
        .clone()
}

fn patch(id: &str) -> &'static str {
    if id == "codex" {
        r#"{"codex_reasoning_effort":"high"}"#
    } else {
        r#"{"claude_effort_level":"high"}"#
    }
}

fn operation(ctx: &Context, command: &str, id: &str, base: &str) -> Envelope {
    execute(
        ctx,
        &request(
            command,
            &[
                "--only",
                &format!("agents:{id}"),
                "--base",
                base,
                "--runtime",
                patch(id),
            ],
        ),
    )
}

fn snapshot(path: &Path) -> Vec<(PathBuf, String)> {
    use sha2::{Digest, Sha256};
    fn visit(root: &Path, path: &Path, out: &mut Vec<(PathBuf, String)>) {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    visit(root, &path, out);
                } else {
                    out.push((
                        path.strip_prefix(root).unwrap().into(),
                        format!("{:x}", Sha256::digest(fs::read(path).unwrap())),
                    ));
                }
            }
        }
    }
    let mut result = Vec::new();
    visit(path, path, &mut result);
    result.sort();
    result
}

#[test]
fn official_main_preview_is_readonly_and_apply_matches_both_main_files() {
    for id in ["claude-code", "codex"] {
        let fixture = Fixture::new();
        fixture.profiles();
        let base = current(&fixture.ctx, id)["base"]
            .as_str()
            .unwrap()
            .to_string();
        let before = snapshot(&fixture.root);
        let preview = operation(&fixture.ctx, "agents:preview-runtime", id, &base);
        assert!(preview.ok, "{:?}", preview.errors);
        assert_eq!(
            snapshot(&fixture.root),
            before,
            "preview must not create locks or a database"
        );
        let applied = operation(&fixture.ctx, "agents:configure", id, &base);
        assert!(applied.ok, "{:?}", applied.errors);
        assert_eq!(
            fs::read(preview.data["path"].as_str().unwrap()).unwrap(),
            preview.data["content"].as_str().unwrap().as_bytes()
        );
        for dir in [".account-a", ".account-b"] {
            assert_eq!(
                fs::read(fixture.ctx.home_dir.join(dir).join("settings.json")).unwrap(),
                CLAUDE
            );
            assert_eq!(
                fs::read(fixture.ctx.home_dir.join(dir).join("config.toml")).unwrap(),
                CODEX
            );
        }
        assert_eq!(
            fs::read(fixture.ctx.home_dir.join(".claude.json")).unwrap(),
            b"{\"dummy\":true}\n"
        );
    }
}

#[test]
fn profile_and_custom_claude_ids_cannot_use_main_configuration_entry_points() {
    let fixture = Fixture::new();
    fixture.profiles();
    let base = current(&fixture.ctx, "claude-code")["base"]
        .as_str()
        .unwrap()
        .to_string();
    fixture.legacy(|policy| {
        policy["agents"]["definitions"]["custom-claude"] =
            policy["agents"]["definitions"]["claude-code"].clone();
    });
    let before = snapshot(&fixture.root);
    for id in ["account-a", "account-b", "custom-claude"] {
        for command in ["agents:preview-runtime", "agents:configure"] {
            let result = operation(&fixture.ctx, command, id, &base);
            assert!(!result.ok, "{command} accepted {id}: {result:?}");
            assert_eq!(
                snapshot(&fixture.root),
                before,
                "refusal changed state for {id}"
            );
            fixture.unchanged();
        }
    }
}

#[test]
fn legacy_main_configuration_cannot_redirect_to_another_account_or_swap_cli_kind() {
    for id in ["claude-code", "codex"] {
        for change in ["path", "kind", "profile", "disabled"] {
            let fixture = Fixture::new();
            fixture.profiles();
            let base = current(&fixture.ctx, id)["base"]
                .as_str()
                .unwrap()
                .to_string();
            fixture.legacy(|policy| match change {
                "path" => {
                    policy["agents"]["definitions"][id]["mcp_config"]["path"] =
                        json!(if id == "codex" {
                            "~/.account-b/config.toml"
                        } else {
                            "~/.account-b/.claude.json"
                        });
                }
                "kind" => {
                    policy["agents"]["definitions"][id]["bin"] =
                        json!(if id == "codex" { "claude" } else { "codex" });
                }
                "profile" => {
                    policy["agents"]["definitions"][id]["profile"] =
                        json!({"base_agent":"claude-code", "config_dir":"~/.account-b"});
                }
                _ => {
                    policy["agents"][id]["enabled"] = json!(false);
                }
            });
            let before = snapshot(&fixture.root);
            assert!(
                current(&fixture.ctx, id)["base"].is_null(),
                "unsafe status gave write baseline"
            );
            for command in ["agents:preview-runtime", "agents:configure"] {
                assert!(
                    !operation(&fixture.ctx, command, id, &base).ok,
                    "accepted {id}/{change}"
                );
                assert_eq!(snapshot(&fixture.root), before);
                fixture.unchanged();
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn linked_main_runtime_file_is_not_read_or_written_and_other_accounts_remain_unchanged() {
    use std::os::unix::fs::symlink;
    for id in ["claude-code", "codex"] {
        for link in ["symlink", "hardlink", "ancestor"] {
            let fixture = Fixture::new();
            let base = current(&fixture.ctx, id)["base"]
                .as_str()
                .unwrap()
                .to_string();
            let (dir, filename, bytes) = if id == "codex" {
                (".codex", "config.toml", CODEX)
            } else {
                (".claude", "settings.json", CLAUDE)
            };
            let target = fixture.ctx.home_dir.join(dir).join(filename);
            let other = fixture.root.join("independent-dummy");
            fs::create_dir(&other).unwrap();
            let original = other.join(filename);
            fs::write(&original, bytes).unwrap();
            if link == "ancestor" {
                fs::remove_dir_all(fixture.ctx.home_dir.join(dir)).unwrap();
                symlink(&other, fixture.ctx.home_dir.join(dir)).unwrap();
            } else {
                fs::remove_file(&target).unwrap();
                if link == "symlink" {
                    symlink(&original, &target).unwrap();
                } else {
                    fs::hard_link(&original, &target).unwrap();
                }
            }
            let before = snapshot(&fixture.root);
            let status = current(&fixture.ctx, id);
            assert_eq!(status["state"], "invalid-target");
            assert!(status["base"].is_null());
            assert!(status["config"].is_null());
            for command in ["agents:preview-runtime", "agents:configure"] {
                assert!(!operation(&fixture.ctx, command, id, &base).ok);
                assert_eq!(snapshot(&fixture.root), before);
                assert_eq!(fs::read(&original).unwrap(), bytes);
                for dir in [".account-a", ".account-b"] {
                    assert_eq!(
                        fs::read(fixture.ctx.home_dir.join(dir).join("settings.json")).unwrap(),
                        CLAUDE
                    );
                }
            }
        }
    }
}

#[test]
fn active_account_session_blocks_runtime_writes_while_preview_remains_readonly() {
    use fs2::FileExt;
    use sha2::{Digest, Sha256};
    for id in ["claude-code", "codex"] {
        let fixture = Fixture::new();
        fixture.profiles();
        let base = current(&fixture.ctx, id)["base"]
            .as_str()
            .unwrap()
            .to_string();
        let dir = fixture
            .ctx
            .home_dir
            .join(if id == "codex" { ".codex" } else { ".claude" });
        let locks = fixture.ctx.data_dir.join("account-locks");
        fs::create_dir_all(&locks).unwrap();
        let hash = format!("{:x}", Sha256::digest(dir.to_string_lossy().as_bytes()));
        let lock = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(locks.join(hash))
            .unwrap();
        FileExt::try_lock_shared(&lock).unwrap();
        let before = snapshot(&fixture.root);
        assert!(operation(&fixture.ctx, "agents:preview-runtime", id, &base).ok);
        assert_eq!(snapshot(&fixture.root), before);
        let refused = operation(&fixture.ctx, "agents:configure", id, &base);
        assert!(!refused.ok, "runtime write ignored active account session");
        assert!(refused.errors[0].message.contains("正在运行"));
        assert_eq!(snapshot(&fixture.root), before);
        fixture.unchanged();
        drop(lock);
        assert!(operation(&fixture.ctx, "agents:configure", id, &base).ok);
    }
}
