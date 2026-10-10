//! Registration and import reject unsafe candidates without changing account files.

use agent_manager_engine::{execute, store, Context, Envelope, Request};
use serde_json::{json, Value};
use std::path::PathBuf;

struct Fixture {
    root: PathBuf,
    ctx: Context,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("am-account-policy-{}", store::uuid_v4()));
        let ctx = Context {
            home_dir: root.join("home"),
            data_dir: root.join("data"),
        };
        for dir in [".claude", ".a", ".b"] {
            std::fs::create_dir_all(ctx.home_dir.join(dir)).unwrap();
            std::fs::write(ctx.home_dir.join(dir).join("settings.json"), b"{}\n").unwrap();
        }
        std::fs::write(ctx.home_dir.join(".claude.json"), b"{}\n").unwrap();
        Self { root, ctx }
    }
    fn legacy(&self, id: &str, value: Value) {
        let mut policy = store::effective_policy(&self.ctx).unwrap();
        policy["agents"]["definitions"][id] = value;
        store::write_policy(&store::open_write(&self.ctx).unwrap(), &policy).unwrap();
    }
    fn run(&self, command: &str, args: Vec<String>) -> Envelope {
        execute(
            &self.ctx,
            &Request {
                command: command.into(),
                args,
                run_id: "policy-isolation-test".into(),
            },
        )
    }
    fn save(&self, id: &str, value: Value) -> Envelope {
        self.run(
            "policy:set",
            vec![
                "--layer".into(),
                "local".into(),
                "--key".into(),
                format!("agents.definitions.{id}"),
                "--value".into(),
                value.to_string(),
            ],
        )
    }
    fn unchanged(&self, before: &Value) {
        assert_eq!(&store::effective_policy(&self.ctx).unwrap(), before);
        for file in [
            ".claude/settings.json",
            ".a/settings.json",
            ".b/settings.json",
            ".claude.json",
        ] {
            assert_eq!(
                std::fs::read(self.ctx.home_dir.join(file)).unwrap(),
                b"{}\n"
            );
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn official(dir: &str) -> Value {
    json!({"bin":"claude", "profile":{"base_agent":"claude-code", "config_dir":dir}, "mcp_config":{"path":format!("{dir}/.claude.json"),"format":"json"}})
}
fn api(dir: &str, variable: &str) -> Value {
    let mut raw = official(dir);
    raw["profile"]["env"] = json!({"ANTHROPIC_API_KEY":format!("${{{variable}}}"), "ANTHROPIC_BASE_URL":"https://api.example.com"});
    raw
}

#[test]
fn registration_rejects_equal_directories_and_shared_secret_ownership() {
    let f = Fixture::new();
    f.legacy("a", api("~/.a", "AM_ACCOUNT_A_KEY"));
    let before = store::effective_policy(&f.ctx).unwrap();
    assert!(!f.save("b", official("~/.a")).ok);
    f.unchanged(&before);
    assert!(!f.save("b", api("~/.b", "AM_ACCOUNT_A_KEY")).ok);
    f.unchanged(&before);
    assert!(f.save("b", official("~/.b")).ok);
}

#[test]
fn unrelated_malformed_definition_does_not_prevent_safe_registration_or_removal() {
    let f = Fixture::new();
    f.legacy(
        "legacy",
        json!({"profile":{"config_dir":"~/.unused", "env":"invalid"}}),
    );
    assert!(f.save("a", official("~/.a")).ok);
    let removed = f.run(
        "policy:delete",
        vec![
            "--layer".into(),
            "local".into(),
            "--key".into(),
            "agents.definitions.legacy".into(),
        ],
    );
    assert!(removed.ok, "{:?}", removed.errors);
}

#[test]
fn import_rejects_colliding_new_accounts_and_preserves_existing_registration() {
    let f = Fixture::new();
    f.legacy("a", official("~/.a"));
    let before = store::effective_policy(&f.ctx).unwrap();
    let mut candidate = before.clone();
    candidate["agents"]["definitions"]["b"] = official("~/.b");
    candidate["agents"]["definitions"]["c"] = official("~/.b");
    let payload = json!({"format":"agent-manager/config-backup/v1", "policy":candidate, "sources":{}, "mcp_saved":[]});
    let result = f.run("config:import", vec!["--value".into(), payload.to_string()]);
    assert!(!result.ok);
    f.unchanged(&before);
}

#[test]
fn registration_generations_survive_edits_and_change_on_recreation() {
    let f = Fixture::new();
    let legacy = official("~/.a");
    f.legacy("a", legacy.clone());
    assert!(
        store::effective_policy(&f.ctx).unwrap()["agents"]["definitions"]["a"]
            .get("account_generation")
            .is_none()
    );
    assert!(
        f.save("a", legacy).ok,
        "explicitly saving unchanged legacy metadata must initialize the generation"
    );
    let first = store::effective_policy(&f.ctx).unwrap()["agents"]["definitions"]["a"].clone();
    let generation = first["account_generation"].as_str().unwrap().to_string();
    assert_eq!(generation.len(), 36);
    let mut renamed = first.clone();
    renamed["display_name"] = json!("Renamed A");
    assert!(f.save("a", renamed.clone()).ok);
    assert_eq!(
        store::effective_policy(&f.ctx).unwrap()["agents"]["definitions"]["a"]
            ["account_generation"],
        generation
    );
    let mut forged = renamed;
    forged["account_generation"] = json!(store::uuid_v4());
    let before = store::effective_policy(&f.ctx).unwrap();
    assert!(!f.save("a", forged).ok);
    f.unchanged(&before);
    assert!(
        !f.run(
            "policy:delete",
            vec![
                "--layer".into(),
                "local".into(),
                "--key".into(),
                "agents.definitions.a.account_generation".into()
            ]
        )
        .ok
    );
    f.unchanged(&before);
    assert!(
        f.run(
            "policy:delete",
            vec![
                "--layer".into(),
                "local".into(),
                "--key".into(),
                "agents.definitions.a.profile".into()
            ]
        )
        .ok
    );
    let recreated = f.run(
        "policy:set",
        vec![
            "--layer".into(),
            "local".into(),
            "--key".into(),
            "agents.definitions.a.profile".into(),
            "--value".into(),
            first["profile"].to_string(),
        ],
    );
    assert!(recreated.ok, "{:?}", recreated.errors);
    let second = store::effective_policy(&f.ctx).unwrap()["agents"]["definitions"]["a"].clone();
    assert_ne!(second["account_generation"], generation);
    assert!(
        f.run(
            "policy:delete",
            vec![
                "--layer".into(),
                "local".into(),
                "--key".into(),
                "agents.definitions.a".into()
            ]
        )
        .ok
    );
    assert!(f.save("a", official("~/.a")).ok);
    assert_ne!(
        store::effective_policy(&f.ctx).unwrap()["agents"]["definitions"]["a"]
            ["account_generation"],
        second["account_generation"]
    );
}

#[test]
fn imported_snapshots_cannot_share_keys_and_recreated_ids_get_local_generations() {
    let f = Fixture::new();
    assert!(f.save("a", api("~/.a", "AM_ACCOUNT_A_KEY")).ok);
    assert!(f.save("b", official("~/.b")).ok);
    let before = store::effective_policy(&f.ctx).unwrap();
    let backup = |policy: Value, rows: Value| json!({"format":"agent-manager/config-backup/v1", "policy":policy, "sources":{}, "mcp_saved":rows});
    let row = |name: &str| json!([{"server":"remote", "agent":"b", "entry":{"type":"http","url":"https://mcp.example.invalid","headers":{"Authorization":format!("${{{name}}}")}},"resolve":{"env":[],"headers":[]},"origin":"disabled"}]);
    let shared = backup(before.clone(), row("AM_ACCOUNT_A_KEY"));
    assert!(
        !f.run("config:import", vec!["--value".into(), shared.to_string()])
            .ok
    );
    f.unchanged(&before);
    let own = backup(before.clone(), row("AM_ACCOUNT_B_MCP_KEY"));
    let imported = f.run("config:import", vec!["--value".into(), own.to_string()]);
    assert!(imported.ok, "{:?}", imported.errors);
    assert_eq!(
        store::effective_policy(&f.ctx).unwrap()["agents"]["definitions"]["a"]
            ["account_generation"],
        before["agents"]["definitions"]["a"]["account_generation"]
    );
    assert!(
        f.run(
            "policy:delete",
            vec![
                "--layer".into(),
                "local".into(),
                "--key".into(),
                "agents.definitions.a".into()
            ]
        )
        .ok
    );
    let retired = f.run(
        "config:import",
        vec![
            "--value".into(),
            backup(before.clone(), json!([])).to_string(),
        ],
    );
    assert!(
        !retired.ok,
        "a deleted registration's variable must stay reserved"
    );
    let mut fresh = before.clone();
    fresh["agents"]["definitions"]["a"]["profile"]["env"]["ANTHROPIC_API_KEY"] =
        json!("${AM_ACCOUNT_A_NEW_KEY}");
    let restored = f.run(
        "config:import",
        vec!["--value".into(), backup(fresh, json!([])).to_string()],
    );
    assert!(restored.ok, "{:?}", restored.errors);
    assert_ne!(
        store::effective_policy(&f.ctx).unwrap()["agents"]["definitions"]["a"]
            ["account_generation"],
        before["agents"]["definitions"]["a"]["account_generation"]
    );
}

#[test]
fn mcp_only_registration_recreation_cannot_reuse_old_variable_ownership() {
    let f = Fixture::new();
    let definition = json!({"mcp_config":{"path":"~/.a/.claude.json","format":"claude-json"}});
    let created = f.save("custom-mcp", definition.clone());
    assert!(created.ok, "{:?}", created.errors);
    let first =
        store::effective_policy(&f.ctx).unwrap()["agents"]["definitions"]["custom-mcp"].clone();
    let backup = |policy: Value, variable: &str| json!({"format":"agent-manager/config-backup/v1","policy":policy,"sources":{},"mcp_saved":[{"server":"remote","agent":"custom-mcp","entry":{"type":"http","url":"https://mcp.example.invalid","headers":{"Authorization":format!("${{{variable}}}")}},"resolve":{},"origin":"disabled"}]});
    let policy = store::effective_policy(&f.ctx).unwrap();
    let registered = f.run(
        "config:import",
        vec![
            "--value".into(),
            backup(policy, "YOUR_CUSTOM_OLD_KEY").to_string(),
        ],
    );
    assert!(registered.ok, "{:?}", registered.errors);
    let removed = f.run(
        "policy:delete",
        vec![
            "--layer".into(),
            "local".into(),
            "--key".into(),
            "agents.definitions.custom-mcp".into(),
        ],
    );
    assert!(removed.ok, "{:?}", removed.errors);
    let before = store::effective_policy(&f.ctx).unwrap();
    let reused = f.save("custom-mcp", definition.clone());
    assert!(
        !reused.ok,
        "a saved snapshot still belongs to the retired registration"
    );
    f.unchanged(&before);
    // Removing the inactive snapshot does not release its durable reservation.
    let conn = store::open_write(&f.ctx).unwrap();
    conn.execute("DELETE FROM mcp_saved WHERE agent='custom-mcp'", [])
        .unwrap();
    drop(conn);
    let recreated = f.save("custom-mcp", definition);
    assert!(recreated.ok, "{:?}", recreated.errors);
    let second = store::effective_policy(&f.ctx).unwrap();
    assert_ne!(
        second["agents"]["definitions"]["custom-mcp"]["account_generation"],
        first["account_generation"]
    );
    assert!(
        !f.run(
            "config:import",
            vec![
                "--value".into(),
                backup(second.clone(), "YOUR_CUSTOM_OLD_KEY").to_string()
            ]
        )
        .ok
    );
    let own = f.run(
        "config:import",
        vec![
            "--value".into(),
            backup(second, "YOUR_CUSTOM_NEW_KEY").to_string(),
        ],
    );
    assert!(own.ok, "{:?}", own.errors);
}

#[test]
fn mcp_only_registration_rejects_native_shared_references_before_creating_database() {
    let f = Fixture::new();
    let entry = json!({"mcpServers":{"private":{"command":"fake-server","env":{"TOKEN":"${YOUR_SHARED_KEY}"}}}});
    let main = f.ctx.home_dir.join(".claude.json");
    let target = f.ctx.home_dir.join(".b/.claude.json");
    std::fs::write(&main, entry.to_string()).unwrap();
    std::fs::write(&target, entry.to_string()).unwrap();
    let before = std::fs::read(&main).unwrap();
    let result = f.save(
        "custom-mcp",
        json!({"mcp_config":{"path":"~/.b/.claude.json","format":"claude-json"}}),
    );
    assert!(!result.ok);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("密钥变量")),
        "{:?}",
        result.errors
    );
    assert!(!f.ctx.data_dir.exists());
    assert_eq!(std::fs::read(main).unwrap(), before);
    assert_eq!(std::fs::read(target).unwrap(), before);
}
