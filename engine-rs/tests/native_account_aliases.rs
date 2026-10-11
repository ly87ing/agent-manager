//! Shell command edits must never read or overwrite account configuration.
use agent_manager_engine::{execute, store, Context, Envelope, Request};
use serde_json::json;
use std::path::PathBuf;

struct Fixture {
    root: PathBuf,
    ctx: Context,
}

impl Fixture {
    fn new() -> Self {
        #[cfg(unix)]
        std::env::set_var("SHELL", "/bin/zsh");
        let root = std::env::temp_dir().join(format!("am-account-aliases-{}", store::uuid_v4()));
        let ctx = Context {
            home_dir: root.join("home"),
            data_dir: root.join("data"),
        };
        std::fs::create_dir_all(&ctx.home_dir).unwrap();
        Self { root, ctx }
    }

    fn run(&self, command: &str, args: &[&str]) -> Envelope {
        execute(
            &self.ctx,
            &Request {
                command: command.into(),
                args: args.iter().map(|s| s.to_string()).collect(),
                run_id: "account-aliases-test".into(),
            },
        )
    }

    fn reserve_disabled_malformed_account(&self) {
        let mut policy = store::builtin_policy();
        policy["agents"]["definitions"]["account-b"] = json!({
            "bin":"codex", "profile":{"base_agent":"codex", "config_dir":"~/.isolated-b", "env":"malformed"}
        });
        policy["agents"]["account-b"] = json!({"enabled":false});
        store::write_policy(&store::open_write(&self.ctx).unwrap(), &policy).unwrap();
    }

    fn blocked_without_writes(&self) {
        let db = self.ctx.data_dir.join("native-manager.db");
        let had_data = self.ctx.data_dir.exists();
        let before = std::fs::read(&db).ok();
        let status = self.run("aliases:status", &[]);
        assert!(status.ok, "{:?}", status.errors);
        #[cfg(unix)]
        let id = "aliases:zshrc";
        #[cfg(windows)]
        let id = "aliases:cmd";
        let target = status.data["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == id)
            .unwrap();
        assert!(target["observed"]["base"].is_null(), "{target}");
        assert!(
            target["observed"]["blocked_reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("冲突")),
            "{target}"
        );
        #[cfg(unix)]
        let base = "--zshrc-base";
        #[cfg(windows)]
        let base = "--cmd-base";
        for command in ["aliases:preview", "aliases:apply"] {
            let result = self.run(
                command,
                &[
                    base,
                    "missing",
                    "--set",
                    "audit-command",
                    "--value",
                    "echo audit",
                ],
            );
            assert!(!result.ok, "{command} accepted protected target");
            assert!(
                result.errors.iter().any(|e| e.message.contains("冲突")),
                "{:?}",
                result.errors
            );
        }
        assert_eq!(std::fs::read(&db).ok(), before);
        if !had_data {
            assert!(!self.ctx.data_dir.exists());
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
#[test]
fn rc_links_cannot_read_or_write_main_registered_or_data_files() {
    for relative in [
        ".codex/config.toml",
        ".claude/settings.json",
        ".claude.json",
        ".isolated-b/config.toml",
        ".codex/sessions/sentinel",
        "../data/sentinel",
    ] {
        let f = Fixture::new();
        if relative.starts_with(".isolated-b") {
            f.reserve_disabled_malformed_account();
        }
        let target = f.ctx.home_dir.join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"protected dummy contents\n").unwrap();
        std::os::unix::fs::symlink(&target, f.ctx.home_dir.join(".zshrc")).unwrap();
        f.blocked_without_writes();
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"protected dummy contents\n"
        );
        assert!(std::fs::symlink_metadata(f.ctx.home_dir.join(".zshrc"))
            .unwrap()
            .file_type()
            .is_symlink());
    }
}

#[cfg(unix)]
#[test]
fn external_identity_links_and_hardlinks_cannot_be_shell_targets() {
    for (relative, hardlink) in [
        (".codex/config.toml", false),
        (".claude.json", false),
        (".isolated-b/auth.json", false),
        (".codex/config.toml", true),
    ] {
        let f = Fixture::new();
        if relative.starts_with(".isolated-b") {
            f.reserve_disabled_malformed_account();
        }
        let target = f.root.join("external-identity");
        std::fs::write(&target, b"protected external dummy\n").unwrap();
        let identity = f.ctx.home_dir.join(relative);
        std::fs::create_dir_all(identity.parent().unwrap()).unwrap();
        if hardlink {
            std::fs::hard_link(&target, &identity).unwrap();
            std::fs::hard_link(&target, f.ctx.home_dir.join(".zshrc")).unwrap();
        } else {
            std::os::unix::fs::symlink(&target, &identity).unwrap();
            std::os::unix::fs::symlink(&target, f.ctx.home_dir.join(".zshrc")).unwrap();
        }
        f.blocked_without_writes();
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"protected external dummy\n"
        );
        assert_eq!(
            std::fs::read(&identity).unwrap(),
            b"protected external dummy\n"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_custom_assistants_external_mcp_configuration_is_protected() {
    let f = Fixture::new();
    let target = f.root.join("custom-assistant.json");
    std::fs::write(&target, b"{\"mcpServers\":{}}\n").unwrap();
    let mut policy = store::builtin_policy();
    policy["agents"]["definitions"]["opencode"]["mcp_config"] = json!(target);
    store::write_policy(&store::open_write(&f.ctx).unwrap(), &policy).unwrap();
    std::os::unix::fs::symlink(&target, f.ctx.home_dir.join(".zshrc")).unwrap();
    f.blocked_without_writes();
    assert_eq!(std::fs::read(&target).unwrap(), b"{\"mcpServers\":{}}\n");
}

#[cfg(unix)]
#[test]
fn redirect_after_status_cannot_reuse_a_missing_block_baseline() {
    let f = Fixture::new();
    let initial = f.run("aliases:status", &[]);
    assert!(initial.ok, "{:?}", initial.errors);
    let elements = initial.data["elements"].as_array().unwrap();
    let target = elements
        .iter()
        .find(|e| e["id"] == "aliases:zshrc")
        .unwrap();
    assert_eq!(target["observed"]["base"], json!("missing"));
    let protected = f.ctx.home_dir.join(".codex/config.toml");
    std::fs::create_dir_all(protected.parent().unwrap()).unwrap();
    std::fs::write(&protected, b"model = 'dummy-model'\n").unwrap();
    std::os::unix::fs::symlink(&protected, f.ctx.home_dir.join(".zshrc")).unwrap();
    f.blocked_without_writes();
    assert_eq!(
        std::fs::read(&protected).unwrap(),
        b"model = 'dummy-model'\n"
    );
}

#[cfg(windows)]
#[test]
fn command_directory_junctions_cannot_enter_account_or_data_directories() {
    for relative in [".codex", ".isolated-b", "../data"] {
        let f = Fixture::new();
        if relative == ".isolated-b" {
            f.reserve_disabled_malformed_account();
        }
        let target = f.ctx.home_dir.join(relative);
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("sentinel"), b"protected dummy contents\n").unwrap();
        let link = f.ctx.home_dir.join(".agent-manager/bin");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        junction::create(&target, &link).unwrap();
        f.blocked_without_writes();
        assert_eq!(
            std::fs::read(target.join("sentinel")).unwrap(),
            b"protected dummy contents\n"
        );
        assert!(!target.join("audit-command.cmd").exists());
    }
}
