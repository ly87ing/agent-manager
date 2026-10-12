//! Account path regressions use only isolated HOME/data and dummy configuration.

use agent_manager_engine::{execute, store, Context, Envelope, Request};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const IDENTITY_FILES: [&str; 8] = [
    "settings.json",
    "settings.local.json",
    "config.toml",
    ".credentials.json",
    "auth.json",
    ".claude.json",
    "managed-settings.json",
    "managed-settings-cache.json",
];
const ORIGINAL: &[u8] = b"{\"model\":\"original\",\"theme\":\"unchanged\"}\n";

struct Fixture {
    root: PathBuf,
    ctx: Context,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("am-account-paths-{tag}-{}", store::uuid_v4()));
        let ctx = Context {
            home_dir: root.join("home"),
            data_dir: root.join("data"),
        };
        std::fs::create_dir_all(&ctx.home_dir).unwrap();
        for dir in [".claude", ".account-a", ".account-b"] {
            std::fs::create_dir_all(ctx.home_dir.join(dir)).unwrap();
            std::fs::write(ctx.home_dir.join(dir).join("settings.json"), ORIGINAL).unwrap();
        }
        std::fs::write(ctx.home_dir.join(".claude.json"), b"{}\n").unwrap();
        Self { root, ctx }
    }

    /// Direct storage represents an unsafe old registration, without relaxing save gates.
    fn legacy(&self, id: &str, dir: &str, kind: &str, malformed: bool, disabled: bool) {
        let mut policy = store::effective_policy(&self.ctx).unwrap();
        let mut definition = json!({"bin":kind,"profile":{"base_agent":kind,"config_dir":dir,"model":"changed"},"_replace":true});
        if malformed {
            definition["profile"]["env"] = json!("invalid");
        }
        policy["agents"]["definitions"][id] = definition;
        if disabled {
            policy["agents"][id] = json!({"enabled":false});
        }
        store::write_policy(&store::open_write(&self.ctx).unwrap(), &policy).unwrap();
    }

    fn unchanged(&self) {
        for dir in [".claude", ".account-a", ".account-b"] {
            assert_eq!(
                std::fs::read(self.ctx.home_dir.join(dir).join("settings.json")).unwrap(),
                ORIGINAL
            );
            assert!(!self
                .ctx
                .home_dir
                .join(dir)
                .join(".settings.json.am-tmp")
                .exists());
        }
        assert_eq!(
            std::fs::read(self.ctx.home_dir.join(".claude.json")).unwrap(),
            b"{}\n"
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn request(command: &str, args: &[&str]) -> Request {
    Request {
        command: command.into(),
        args: args.iter().map(|arg| (*arg).into()).collect(),
        run_id: "account-path-test".into(),
    }
}

fn status(ctx: &Context, id: &str) -> Value {
    let result = execute(ctx, &request("rules:status", &["--offline"]));
    assert!(result.ok, "{:?}", result.errors);
    result.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == format!("rules:profile:{id}"))
        .unwrap()
        .clone()
}

fn apply(ctx: &Context, id: &str) -> Envelope {
    execute(
        ctx,
        &request(
            "rules:apply",
            &["--only", &format!("profiles:{id}"), "--base", "missing"],
        ),
    )
}

fn assert_blocked(fixture: &Fixture, id: &str) {
    let policy = store::effective_policy(&fixture.ctx).unwrap();
    let item = status(&fixture.ctx, id);
    assert_eq!(item["state"], "unmanaged-detected", "{item}");
    assert!(item["observed"]["base"].is_null());
    assert!(item["actions"].as_array().unwrap().is_empty());
    assert!(!apply(&fixture.ctx, id).ok, "unsafe operation was accepted");
    assert_eq!(store::effective_policy(&fixture.ctx).unwrap(), policy);
    fixture.unchanged();
}

#[test]
fn legacy_equal_directories_remain_reserved_across_kind_disabled_and_malformed_definitions() {
    for (kind, malformed, disabled) in [
        ("claude", false, false),
        ("codex", false, false),
        ("claude", false, true),
        ("codex", true, true),
    ] {
        let fixture = Fixture::new("equal-directory");
        fixture.legacy("account-a", "~/.account-a", "claude", false, false);
        fixture.legacy("account-b", "~/.account-a", kind, malformed, disabled);
        assert_blocked(&fixture, "account-a");
        if !malformed && !disabled {
            assert_eq!(
                status(&fixture.ctx, "account-b")["state"],
                "unmanaged-detected"
            );
        }
    }
}

#[test]
fn every_identity_file_rejects_hardlink_aliases_before_status_or_apply() {
    for name in IDENTITY_FILES {
        let fixture = Fixture::new("hardlink");
        fixture.legacy("account-a", "~/.account-a", "claude", false, false);
        let protected = fixture.root.join("independent-dummy.json");
        std::fs::write(&protected, b"{}\n").unwrap();
        let target = fixture.ctx.home_dir.join(".account-a").join(name);
        if target.exists() {
            std::fs::remove_file(&target).unwrap();
        }
        std::fs::hard_link(&protected, &target).unwrap();
        let item = status(&fixture.ctx, "account-a");
        assert_eq!(
            item["state"], "unmanaged-detected",
            "identity alias {name} was accepted: {item}"
        );
        assert!(!apply(&fixture.ctx, "account-a").ok);
        assert_eq!(std::fs::read(&protected).unwrap(), b"{}\n");
        assert_eq!(std::fs::read(&target).unwrap(), b"{}\n");
        assert_eq!(
            std::fs::read(fixture.ctx.home_dir.join(".claude/settings.json")).unwrap(),
            ORIGINAL
        );
        assert_eq!(
            std::fs::read(fixture.ctx.home_dir.join(".account-b/settings.json")).unwrap(),
            ORIGINAL
        );
        assert!(!target
            .parent()
            .unwrap()
            .join(".settings.json.am-tmp")
            .exists());
    }
}

#[cfg(unix)]
#[test]
fn every_identity_file_rejects_main_other_independent_and_dangling_links() {
    for name in IDENTITY_FILES {
        for source in ["main", "other", "independent", "dangling"] {
            let fixture = Fixture::new("identity-link");
            fixture.legacy("account-a", "~/.account-a", "claude", false, false);
            fixture.legacy("account-b", "~/.account-b", "codex", false, false);
            let protected = match source {
                "main" => fixture.ctx.home_dir.join(".claude.json"),
                "other" => fixture.ctx.home_dir.join(".account-b/settings.json"),
                _ => fixture.root.join("independent-dummy.json"),
            };
            if source == "independent" {
                std::fs::write(&protected, b"{}\n").unwrap();
            }
            let target = fixture.ctx.home_dir.join(".account-a").join(name);
            if target.exists() {
                std::fs::remove_file(&target).unwrap();
            }
            std::os::unix::fs::symlink(&protected, &target).unwrap();
            let before = std::fs::read(&protected).ok();
            let item = status(&fixture.ctx, "account-a");
            assert_eq!(
                item["state"], "unmanaged-detected",
                "{name}/{source}: {item}"
            );
            assert!(item["observed"]["base"].is_null());
            assert!(!apply(&fixture.ctx, "account-a").ok);
            assert_eq!(std::fs::read(&protected).ok(), before);
            assert!(std::fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(
                std::fs::read(fixture.ctx.home_dir.join(".account-b/settings.json")).unwrap(),
                ORIGINAL
            );
            assert_eq!(
                std::fs::read(fixture.ctx.home_dir.join(".claude/settings.json")).unwrap(),
                ORIGINAL
            );
            assert!(!fixture
                .ctx
                .home_dir
                .join(".account-a/.settings.json.am-tmp")
                .exists());
        }
    }
}

fn alias_dir(target: &Path, alias: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, alias).unwrap();
    #[cfg(windows)]
    junction::create(target, alias).unwrap();
}

#[test]
fn ordinary_independent_ancestor_alias_is_rejected_even_with_missing_suffix() {
    let fixture = Fixture::new("ancestor");
    let ordinary = fixture.ctx.home_dir.join("ordinary");
    std::fs::create_dir_all(&ordinary).unwrap();
    alias_dir(&ordinary, &fixture.ctx.home_dir.join("alias"));
    fixture.legacy(
        "account-a",
        "~/alias/not-created/deep",
        "claude",
        false,
        false,
    );
    assert_blocked(&fixture, "account-a");
    assert!(!ordinary.join("not-created").exists());
}

#[test]
fn trusted_home_alias_and_shared_resource_links_do_not_create_identity_aliases() {
    let fixture = Fixture::new("trusted-home");
    let home_alias = fixture.root.join("home-alias");
    alias_dir(&fixture.ctx.home_dir, &home_alias);
    let context = Context {
        home_dir: home_alias,
        data_dir: fixture.ctx.data_dir.clone(),
    };
    let skills = fixture.root.join("shared-skills");
    std::fs::create_dir_all(&skills).unwrap();
    alias_dir(&skills, &fixture.ctx.home_dir.join(".account-a/skills"));
    fixture.legacy("account-a", "~/.account-a", "claude", false, false);
    fixture.legacy("account-b", "~/.account-b", "codex", false, false);
    for id in ["account-a", "account-b"] {
        let item = status(&context, id);
        assert_ne!(
            item["state"], "unmanaged-detected",
            "ordinary account was rejected: {item}"
        );
        assert!(item["observed"]["base"].is_string());
    }
    fixture.unchanged();
}
