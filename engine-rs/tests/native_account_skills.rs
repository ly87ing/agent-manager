//! Actual skill writes must stay in the selected account's directory.
use agent_manager_engine::{execute, store, Context, Envelope, Request};
use serde_json::{json, Value};
use std::path::PathBuf;

struct Fixture {
    root: PathBuf,
    ctx: Context,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("am-account-skills-{}", store::uuid_v4()));
        let ctx = Context {
            home_dir: root.join("home"),
            data_dir: root.join("data"),
        };
        std::fs::create_dir_all(&ctx.home_dir).unwrap();
        std::fs::create_dir_all(root.join("source/alpha")).unwrap();
        std::fs::write(root.join("source/alpha/SKILL.md"), "# Dummy skill\n").unwrap();
        for dir in [".claude", ".codex", ".isolated-a", ".isolated-b"] {
            std::fs::create_dir_all(ctx.home_dir.join(dir)).unwrap();
            std::fs::write(ctx.home_dir.join(dir).join("sentinel"), dir).unwrap();
        }
        Self { root, ctx }
    }
    fn run(&self, command: &str, args: &[&str]) -> Envelope {
        execute(
            &self.ctx,
            &Request {
                command: command.into(),
                args: args.iter().map(|s| s.to_string()).collect(),
                run_id: "account-skills-test".into(),
            },
        )
    }
    fn policy(&self, target: Option<&str>) -> Value {
        let mut policy = store::builtin_policy();
        policy["agents"]["definitions"]["account-a"] = json!({"bin":"claude", "profile":{"base_agent":"claude-code", "config_dir":"~/.isolated-a"}});
        policy["agents"]["definitions"]["account-b"] =
            json!({"bin":"codex", "profile":{"base_agent":"codex", "config_dir":"~/.isolated-b"}});
        if let Some(target) = target {
            policy["agents"]["definitions"]["account-a"]["skills_dir"] = json!(target);
        }
        policy["skills"]["repositories"]["audit"] =
            json!({"source_path":self.root.join("source"), "skill_paths":["alpha"]});
        policy
    }
    fn legacy(&self, policy: &Value) {
        store::write_policy(&store::open_write(&self.ctx).unwrap(), policy).unwrap();
    }
    fn apply(&self, agent: &str) -> Envelope {
        self.run(
            "skills:apply",
            &[
                "--only",
                "skill:audit/alpha",
                "--action",
                "add",
                "--agent",
                agent,
            ],
        )
    }
    fn unchanged(&self) {
        for dir in [".claude", ".codex", ".isolated-a", ".isolated-b"] {
            assert_eq!(
                std::fs::read_to_string(self.ctx.home_dir.join(dir).join("sentinel")).unwrap(),
                dir
            );
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn omitted_skill_target_uses_each_profile_directory_and_keeps_other_accounts_unchanged() {
    let f = Fixture::new();
    f.legacy(&f.policy(None));
    let status = f.run("skills:status", &[]);
    assert!(status.ok, "{:?}", status.errors);
    assert!(!f.ctx.home_dir.join(".isolated-a/skills").exists());
    for (id, dir) in [("account-a", ".isolated-a"), ("account-b", ".isolated-b")] {
        let result = f.apply(id);
        assert!(result.ok, "{:?}", result.errors);
        assert_eq!(
            std::fs::read_to_string(f.ctx.home_dir.join(dir).join("skills/alpha/SKILL.md"))
                .unwrap(),
            "# Dummy skill\n"
        );
    }
    assert!(!f.ctx.home_dir.join(".account-a").exists());
    assert!(!f.ctx.home_dir.join(".account-b").exists());
    assert!(!f.ctx.home_dir.join(".claude/skills").exists());
    assert!(!f.ctx.home_dir.join(".codex/skills").exists());
    f.unchanged();
}

#[test]
fn registration_and_import_refuse_foreign_skill_targets_before_creating_data() {
    for target in [
        "~/.codex/foreign-skills",
        "~/.claude/foreign-skills",
        "~/.isolated-b/foreign-skills",
        "~/elsewhere",
    ] {
        let f = Fixture::new();
        let policy = f.policy(Some(target));
        let definition = policy["agents"]["definitions"]["account-a"].to_string();
        let result = f.run(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.account-a",
                "--value",
                &definition,
            ],
        );
        assert!(!result.ok, "accepted {target}");
        assert!(
            result.errors.iter().any(|e| e.code == "FEATURE_RETIRED"),
            "{:?}",
            result.errors
        );
        assert!(!f.ctx.data_dir.exists());
        let payload = json!({"format":"agent-manager/config-backup/v1", "policy":policy, "sources":{}, "mcp_saved":[]}).to_string();
        let imported = f.run("config:import", &["--value", &payload]);
        assert!(!imported.ok, "import accepted {target}");
        assert!(
            imported
                .errors
                .iter()
                .any(|e| e.message.contains("账号定义只读")),
            "{:?}",
            imported.errors
        );
        assert!(!f.ctx.data_dir.exists());
        f.unchanged();
    }
}

#[test]
fn legacy_foreign_targets_block_add_collection_and_cleanup_without_file_changes() {
    let f = Fixture::new();
    f.legacy(&f.policy(Some("~/.codex/foreign-skills")));
    let before = std::fs::read(store::db_path(&f.ctx)).unwrap();
    for (command, args) in [
        (
            "skills:apply",
            vec![
                "--only",
                "skill:audit/alpha",
                "--action",
                "add",
                "--agent",
                "account-a",
            ],
        ),
        (
            "skills:apply",
            vec!["--only", "skill:audit", "--action", "disable"],
        ),
        (
            "skills:apply",
            vec![
                "--action",
                "remove-link",
                "--agent",
                "account-a",
                "--link",
                "alpha",
            ],
        ),
    ] {
        let result = f.run(command, &args);
        assert!(!result.ok, "{command}: {args:?}");
        assert!(
            result.errors.iter().any(|e| e.message.contains("技能")),
            "{:?}",
            result.errors
        );
        assert_eq!(std::fs::read(store::db_path(&f.ctx)).unwrap(), before);
    }
    assert!(!f.ctx.home_dir.join(".codex/foreign-skills").exists());
    f.unchanged();
}

#[test]
fn skill_directory_alias_to_another_account_is_rejected_before_add_or_cleanup() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.ctx.home_dir.join(".isolated-b/skills")).unwrap();
    agent_manager_engine::platform::link_dir(
        &f.ctx.home_dir.join(".isolated-b/skills"),
        &f.ctx.home_dir.join(".isolated-a/skills"),
    )
    .unwrap();
    f.legacy(&f.policy(Some("~/.isolated-a/skills")));
    let result = f.apply("account-a");
    assert!(!result.ok);
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.message.contains("链接") || e.message.contains("联接")),
        "{:?}",
        result.errors
    );
    assert!(!f.ctx.home_dir.join(".isolated-b/skills/alpha").exists());
    let cleanup = f.run(
        "skills:apply",
        &[
            "--action",
            "remove-link",
            "--agent",
            "account-a",
            "--link",
            "alpha",
        ],
    );
    assert!(!cleanup.ok);
    assert!(
        cleanup
            .errors
            .iter()
            .any(|e| e.message.contains("链接") || e.message.contains("联接")),
        "{:?}",
        cleanup.errors
    );
    f.unchanged();
}

#[test]
fn main_and_custom_assistants_cannot_redirect_skill_writes_into_other_accounts() {
    for (id, target) in [
        ("claude-code", "~/.codex/foreign-skills"),
        ("codex", "~/.isolated-a/foreign-skills"),
        ("custom", "~/.claude/foreign-skills"),
        ("custom", "~/.isolated-b/foreign-skills"),
        ("custom", "~/.agents/skills"),
    ] {
        let f = Fixture::new();
        let mut policy = f.policy(None);
        if id == "custom" {
            policy["agents"]["definitions"][id] = json!({"bin":"custom-cli"});
        }
        policy["agents"]["definitions"][id]["skills_dir"] = json!(target);
        f.legacy(&policy);
        let result = f.apply(id);
        assert!(!result.ok, "accepted {id}: {target}");
        assert!(!f
            .ctx
            .home_dir
            .join(target.trim_start_matches("~/"))
            .exists());
        f.unchanged();
    }
}

#[test]
fn legacy_skill_fallback_changes_are_validated_and_custom_targets_remain_supported() {
    let f = Fixture::new();
    f.legacy(&f.policy(None));
    let before = store::effective_policy(&f.ctx).unwrap();
    let rejected = f.run(
        "policy:set",
        &[
            "--layer",
            "local",
            "--key",
            "skills.agents.account-a",
            "--value",
            "\"~/.codex/foreign-skills\"",
        ],
    );
    assert!(!rejected.ok);
    assert!(
        rejected.errors.iter().any(|e| e.message.contains("技能")),
        "{:?}",
        rejected.errors
    );
    assert_eq!(store::effective_policy(&f.ctx).unwrap(), before);
    let mut policy = before;
    policy["agents"]["definitions"]["custom"] =
        json!({"bin":"custom-cli", "skills_dir":f.root.join("custom-skills")});
    f.legacy(&policy);
    let added = f.apply("custom");
    assert!(added.ok, "{:?}", added.errors);
    assert_eq!(
        std::fs::read_to_string(f.root.join("custom-skills/alpha/SKILL.md")).unwrap(),
        "# Dummy skill\n"
    );
    f.unchanged();
}
