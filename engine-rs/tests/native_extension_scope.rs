//! The public tool manages extensions, never account identity or lifecycle.
#[cfg(windows)]
mod common;

use agent_manager_engine::{execute, store, Context, Envelope, Request};
use serde_json::{json, Value};
use std::path::PathBuf;

struct Fixture {
    root: PathBuf,
    ctx: Context,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("am-extension-scope-{}", store::uuid_v4()));
        Self {
            ctx: Context {
                home_dir: root.join("home"),
                data_dir: root.join("data"),
            },
            root,
        }
    }
    fn run(&self, command: &str, args: &[&str]) -> Envelope {
        execute(
            &self.ctx,
            &Request {
                command: command.into(),
                args: args.iter().map(|s| (*s).into()).collect(),
                run_id: "extension-scope".into(),
            },
        )
    }
    fn cli(&self, command: &str, args: &[&str]) -> Value {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
            .env_clear()
            .env("HOME", &self.ctx.home_dir)
            .env("USERPROFILE", &self.ctx.home_dir)
            .env("AGENT_MANAGER_DATA_DIR", &self.ctx.data_dir)
            .env("PATH", "/usr/bin:/bin")
            .arg(command)
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(output.status.success() && value["ok"] == true, "{value}");
        value["data"].clone()
    }
    fn seed(&self) {
        #[cfg(windows)]
        common::private_fixture_root(&self.root);
        std::fs::create_dir_all(self.ctx.home_dir.join(".claude")).unwrap();
        std::fs::create_dir_all(self.ctx.home_dir.join(".codex")).unwrap();
        std::fs::create_dir_all(self.ctx.home_dir.join(".isolated-a")).unwrap();
        std::fs::create_dir_all(self.ctx.home_dir.join(".isolated-b")).unwrap();
        for dir in [".claude", ".codex", ".isolated-a", ".isolated-b"] {
            for name in [
                "auth.json",
                ".credentials.json",
                "settings.json",
                "session-sentinel",
            ] {
                std::fs::write(
                    self.ctx.home_dir.join(dir).join(name),
                    format!("{dir}:{name}:preserve\n"),
                )
                .unwrap();
            }
        }
        std::fs::write(self.ctx.home_dir.join(".claude.json"), b"{\"oauthAccount\": {\"emailAddress\": \"dummy@example.invalid\"}, \"mcpServers\":{}, \"theme\":\"keep\"}\n").unwrap();
        std::fs::write(
            self.ctx.home_dir.join(".isolated-a/.claude.json"),
            b"{ \"oauthAccount\" : { \"id\" : \"A\" }, \"mcpServers\":{}, \"other\" : 2 }\n",
        )
        .unwrap();
        std::fs::write(
            self.ctx.home_dir.join(".codex/config.toml"),
            "# Keep every byte outside MCP\nmodel = 'dummy-model'\nmodel_provider = 'openai'\n",
        )
        .unwrap();
        std::fs::write(
            self.ctx.home_dir.join(".isolated-b/config.toml"),
            "model = 'b-model'\n",
        )
        .unwrap();
        std::fs::create_dir_all(self.root.join("source/alpha")).unwrap();
        std::fs::write(self.root.join("source/alpha/SKILL.md"), "# Alpha\n").unwrap();
        let mut policy = store::builtin_policy();
        policy["agents"]["definitions"]["a"] = json!({"display_name":"Resource A", "bin":"claude", "profile":{"base_agent":"claude-code", "config_dir":"~/.isolated-a"}, "mcp_config":{"path":"~/.isolated-a/.claude.json", "format":"json"}});
        policy["agents"]["definitions"]["b"] = json!({"display_name":"Resource B", "bin":"codex", "profile":{"base_agent":"codex", "config_dir":"~/.isolated-b"}, "mcp_config":{"path":"~/.isolated-b/config.toml", "format":"toml"}});
        policy["skills"]["repositories"]["scope"] =
            json!({"source_path":self.root.join("source"), "skill_paths":["alpha"]});
        store::write_policy(&store::open_write(&self.ctx).unwrap(), &policy).unwrap();
    }
    fn identity(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        for dir in [".claude", ".codex", ".isolated-a", ".isolated-b"] {
            for name in [
                "auth.json",
                ".credentials.json",
                "settings.json",
                "session-sentinel",
            ] {
                let path = self.ctx.home_dir.join(dir).join(name);
                files.push((path.clone(), std::fs::read(path).unwrap()));
            }
        }
        files
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn retired_account_commands_stop_before_home_data_or_cli_access() {
    let f = Fixture::new();
    for command in [
        "accounts:command",
        "profiles:status",
        "profiles:preview",
        "profiles:create",
        "profiles:apply",
        "profiles:apply-definition",
        "profiles:adopt",
        "profiles:seed",
        "profiles:shared-status",
        "profiles:shared-apply",
        "agents:status",
        "agents:auth-status",
        "agents:configure",
        "agents:preview-runtime",
        "agents:apply",
        "aliases:status",
        "aliases:apply",
        "secrets:migrate-legacy",
    ] {
        for command in [command.to_string(), command.replace(':', ".")] {
            let result = f.run(&command, &["--only", "agents:codex", "--probe"]);
            assert!(!result.ok, "{command}");
            assert_eq!(result.errors[0].code, "FEATURE_RETIRED", "{command}");
            assert!(!result.lock.held);
        }
    }
    for key in [
        "agents",
        "agents.definitions.a",
        "agents.definitions.a.profile.env",
        "agents.codex.enabled",
        "aliases",
    ] {
        for command in ["policy:set", "policy:delete"] {
            let result = f.run(
                command,
                &["--layer", "local", "--key", key, "--value", "{}"],
            );
            assert_eq!(result.errors[0].code, "FEATURE_RETIRED");
        }
    }
    assert!(agent_manager_engine::native_account_context::run_cli(&[])
        .unwrap_err()
        .to_string()
        .contains("已移除"));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
        .arg("--account-launch")
        .arg("--home")
        .arg(&f.ctx.home_dir)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("已移除"));
    assert!(!f.root.exists());
}

#[test]
fn resource_labels_do_not_read_invalid_authentication_files_or_initialize_generations() {
    let f = Fixture::new();
    f.seed();
    let before = f.identity();
    let policy = store::effective_policy(&f.ctx).unwrap();
    let result = f.run("resources:targets", &[]);
    assert!(result.ok, "{:?}", result.errors);
    assert!(result.data["targets"]
        .as_array()
        .unwrap()
        .iter()
        .any(|target| target["id"] == "a" && target["name"] == "Resource A"));
    assert_eq!(policy, store::effective_policy(&f.ctx).unwrap());
    assert_eq!(before, f.identity());
    assert!(!result.data.to_string().contains("preserve"));
}

#[test]
fn extension_writes_preserve_account_files_and_unselected_targets() {
    let f = Fixture::new();
    f.seed();
    // Valid assistant settings are unnecessary for extension-only path validation.
    for dir in [".claude", ".isolated-a"] {
        std::fs::write(
            f.ctx.home_dir.join(dir).join("settings.json"),
            "{\"env\":{\"ANTHROPIC_API_KEY\":\"dummy-keep-value\"},\"model\":\"unchanged\"}\n",
        )
        .unwrap();
    }
    let identity = f.identity();
    let policy = store::effective_policy(&f.ctx).unwrap();
    let main = std::fs::read(f.ctx.home_dir.join(".claude.json")).unwrap();
    let codex = std::fs::read(f.ctx.home_dir.join(".codex/config.toml")).unwrap();
    let b = std::fs::read(f.ctx.home_dir.join(".isolated-b/config.toml")).unwrap();
    let path = f.ctx.home_dir.join(".isolated-a/.claude.json");
    let original = std::fs::read_to_string(&path).unwrap();
    f.cli(
        "mcp:apply",
        &[
            "--only",
            "mcp:scope",
            "--agent",
            "a",
            "--action",
            "add",
            "--spec",
            "{\"command\":\"dummy-mcp\"}",
        ],
    );
    let changed = std::fs::read_to_string(&path).unwrap();
    assert!(changed.starts_with(original.split("\"mcpServers\":").next().unwrap()));
    assert!(changed.ends_with(", \"other\" : 2 }\n"));
    assert_eq!(
        serde_json::from_str::<Value>(&changed).unwrap()["oauthAccount"],
        json!({"id":"A"})
    );
    let status = f.run("rules:status", &[]);
    assert!(status.ok, "{:?}", status.errors);
    let base = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "rules:profile:a")
        .unwrap()["observed"]["base"]
        .as_str()
        .unwrap();
    f.cli(
        "rules:apply",
        &[
            "--only",
            "rules:profile:a",
            "--base",
            base,
            "--value",
            "# Resource rules\n",
        ],
    );
    assert_eq!(
        std::fs::read_to_string(f.ctx.home_dir.join(".isolated-a/CLAUDE.md")).unwrap(),
        "# Resource rules\n"
    );
    f.cli(
        "skills:apply",
        &[
            "--only",
            "skill:scope/alpha",
            "--action",
            "add",
            "--agent",
            "a",
        ],
    );
    assert_eq!(
        std::fs::read_to_string(f.ctx.home_dir.join(".isolated-a/skills/alpha/SKILL.md")).unwrap(),
        "# Alpha\n"
    );
    assert!(!f.ctx.home_dir.join(".isolated-b/skills").exists());
    assert_eq!(
        main,
        std::fs::read(f.ctx.home_dir.join(".claude.json")).unwrap()
    );
    assert_eq!(
        codex,
        std::fs::read(f.ctx.home_dir.join(".codex/config.toml")).unwrap()
    );
    assert_eq!(
        b,
        std::fs::read(f.ctx.home_dir.join(".isolated-b/config.toml")).unwrap()
    );
    assert_eq!(identity, f.identity());
    assert_eq!(
        policy["agents"],
        store::effective_policy(&f.ctx).unwrap()["agents"]
    );
}

#[test]
fn backup_import_cannot_add_edit_remove_or_initialize_accounts() {
    let f = Fixture::new();
    f.seed();
    let before = store::effective_policy(&f.ctx).unwrap();
    let identity = f.identity();
    let exported = f.run("config:export", &[]);
    assert!(exported.ok, "{:?}", exported.errors);
    assert!(exported.data["policy"].get("agents").is_none());
    for change in ["add", "edit", "remove"] {
        let mut payload = exported.data.clone();
        payload["policy"]["agents"] = before["agents"].clone();
        match change {
            "add" => {
                payload["policy"]["agents"]["definitions"]["new"] =
                    before["agents"]["definitions"]["a"].clone()
            }
            "edit" => {
                payload["policy"]["agents"]["definitions"]["a"]["display_name"] = json!("changed")
            }
            _ => {
                payload["policy"]["agents"]["definitions"]
                    .as_object_mut()
                    .unwrap()
                    .remove("a");
            }
        }
        assert!(
            !f.run("config:import", &["--value", &payload.to_string()])
                .ok
        );
        assert_eq!(before, store::effective_policy(&f.ctx).unwrap());
        assert_eq!(identity, f.identity());
    }
    let mut alias_backup = exported.data.clone();
    alias_backup["policy"]["aliases"] = json!({"launch":"dummy"});
    assert!(
        !f.run("config:import", &["--value", &alias_backup.to_string()])
            .ok
    );
    assert_eq!(before, store::effective_policy(&f.ctx).unwrap());
    let result = f.run("config:import", &["--value", &exported.data.to_string()]);
    assert!(result.ok, "{:?}", result.errors);
    assert_eq!(
        before["agents"],
        store::effective_policy(&f.ctx).unwrap()["agents"]
    );
    assert_eq!(identity, f.identity());
}

#[test]
fn shared_configuration_changes_only_the_extension_container() {
    for original in [
        "{ \"oauthAccount\":{\"id\":\"dummy\"}, \"theme\":\"keep\" }\n",
        "{\"mcpServers\":null, \"oauthAccount\":{\"id\":\"dummy\"}}\n",
        "{\"oauthAccount\":{\"id\":\"dummy\"},\"mcpServers\":{\"other\":{\"command\":\"keep\"}}}\n",
    ] {
        let f = Fixture::new();
        f.seed();
        let path = f.ctx.home_dir.join(".claude.json");
        std::fs::write(&path, original).unwrap();
        let identity = f.identity();
        f.cli(
            "mcp:apply",
            &[
                "--only",
                "mcp:new",
                "--agent",
                "claude-code",
                "--action",
                "add",
                "--spec",
                "{\"command\":\"dummy-mcp\"}",
            ],
        );
        let changed = std::fs::read_to_string(&path).unwrap();
        let before: Value = serde_json::from_str(original).unwrap();
        let after: Value = serde_json::from_str(&changed).unwrap();
        assert_eq!(before["oauthAccount"], after["oauthAccount"]);
        let key = "\"mcpServers\":";
        if let Some(at) = original.find(key) {
            assert_eq!(&changed[..at + key.len()], &original[..at + key.len()]);
            let raw_before: std::collections::BTreeMap<String, &serde_json::value::RawValue> =
                serde_json::from_str(original).unwrap();
            let raw_after: std::collections::BTreeMap<String, &serde_json::value::RawValue> =
                serde_json::from_str(&changed).unwrap();
            let tail_before = raw_before["mcpServers"].get().as_ptr() as usize
                - original.as_ptr() as usize
                + raw_before["mcpServers"].get().len();
            let tail_after = raw_after["mcpServers"].get().as_ptr() as usize
                - changed.as_ptr() as usize
                + raw_after["mcpServers"].get().len();
            assert_eq!(&original[tail_before..], &changed[tail_after..]);
        } else {
            assert!(changed.starts_with(original.strip_suffix("}\n").unwrap()));
            assert!(changed.ends_with("}\n"));
        }
        assert_eq!(identity, f.identity());
    }
    let f = Fixture::new();
    f.seed();
    let path = f.ctx.home_dir.join(".codex/config.toml");
    let original = std::fs::read_to_string(&path).unwrap();
    f.cli(
        "mcp:apply",
        &[
            "--only",
            "mcp:new",
            "--agent",
            "codex",
            "--action",
            "add",
            "--spec",
            "{\"command\":\"dummy-mcp\"}",
        ],
    );
    assert!(std::fs::read_to_string(path)
        .unwrap()
        .starts_with(&original));
}

#[test]
fn extension_secrets_cannot_edit_account_variables_or_use_account_files() {
    let f = Fixture::new();
    f.seed();
    let source = f.root.join("mcp.env");
    std::fs::write(
        &source,
        "MCP_ONLY=dummy-mcp\nACCOUNT_KEY=dummy-account\nUNUSED=dummy-unused\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    f.cli("secrets:source", &["--path", source.to_str().unwrap()]);
    let mut policy = store::effective_policy(&f.ctx).unwrap();
    policy["agents"]["definitions"]["a"]["profile"]["env"] =
        json!({"ANTHROPIC_AUTH_TOKEN":"${ACCOUNT_KEY}"});
    store::write_policy(&store::open_write(&f.ctx).unwrap(), &policy).unwrap();
    let protected = [
        "ACCOUNT_KEY",
        "ANTHROPIC_API_KEY",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "ANTHROPIC_IDENTITY_TOKEN",
        "OPENAI_API_KEY",
        "AWS_ACCESS_KEY_ID",
        "AZURE_API_KEY",
        "GOOGLE_APPLICATION_CREDENTIALS",
        "CODEX_HOME",
    ];
    let mut env = json!({"T":"${MCP_ONLY}"});
    for (index, name) in protected.iter().enumerate() {
        env[format!("TOKEN_{index}")] = json!(format!("${{{name}}}"));
    }
    std::fs::write(
        f.ctx.home_dir.join(".claude.json"),
        json!({"mcpServers":{"extension":{"command":"dummy","env":env}}}).to_string(),
    )
    .unwrap();
    let before = std::fs::read(&source).unwrap();
    for name in protected.iter().copied().chain(std::iter::once("UNUSED")) {
        assert!(
            !f.run("secrets:set", &["--var", name, "--value", "dummy-changed"])
                .ok
        );
        assert_eq!(before, std::fs::read(&source).unwrap());
    }
    for name in protected {
        let spec = json!({"command":"dummy", "env":{"TOKEN":format!("${{{name}}}")}}).to_string();
        let result = f.run(
            "mcp:apply",
            &[
                "--only",
                "mcp:blocked",
                "--agent",
                "claude-code",
                "--action",
                "add",
                "--spec",
                &spec,
            ],
        );
        assert!(!result.ok);
        assert!(
            result.errors[0]
                .message
                .contains("账号凭据不能用于扩展操作"),
            "{:?}",
            result.errors
        );
        assert_eq!(before, std::fs::read(&source).unwrap());
    }
    f.cli(
        "secrets:set",
        &["--var", "MCP_ONLY", "--value", "dummy-updated"],
    );
    let changed = std::fs::read_to_string(&source).unwrap();
    assert!(changed.contains("dummy-updated"));
    assert!(changed.contains("ACCOUNT_KEY=dummy-account\nUNUSED=dummy-unused\n"));
    let identity = f.identity();
    for path in [
        f.ctx.home_dir.join(".claude/private.env"),
        f.ctx.home_dir.join(".isolated-a/auth.json"),
    ] {
        assert!(
            !f.run(
                "secrets:source",
                &["--path", path.to_str().unwrap(), "--create"]
            )
            .ok
        );
    }
    assert!(!f.ctx.home_dir.join(".claude/private.env").exists());
    assert_eq!(identity, f.identity());
    assert_eq!(
        policy["agents"],
        store::effective_policy(&f.ctx).unwrap()["agents"]
    );
}

#[test]
#[cfg(any(unix, windows))]
fn rule_hardlinks_to_identity_files_are_blocked_before_read_or_write() {
    let f = Fixture::new();
    f.seed();
    let identity = f.ctx.home_dir.join(".claude/auth.json");
    let target = f.ctx.home_dir.join(".claude/CLAUDE.md");
    std::fs::hard_link(&identity, &target).unwrap();
    let before = std::fs::read(&identity).unwrap();
    let status = f.run("rules:status", &[]);
    assert!(status.ok);
    let item = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "rules:claude_global")
        .unwrap();
    assert!(item["observed"]["content"].is_null());
    assert!(item["observed"]["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("硬链接"));
    assert!(
        !f.run(
            "rules:apply",
            &[
                "--only",
                "rules:claude_global",
                "--base",
                "missing",
                "--value",
                "# No write"
            ]
        )
        .ok
    );
    assert_eq!(before, std::fs::read(identity).unwrap());
}
