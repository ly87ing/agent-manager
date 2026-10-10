//! Real engine/terminal entry regressions. Every assistant is a fake executable
//! in an isolated HOME; these tests never call an installed assistant or service.
#![cfg(unix)]

use agent_manager_engine::{execute, Context, Request};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ENGINE: &str = env!("CARGO_BIN_EXE_agent-manager-engine");
const FAKE: &str = r#"#!/bin/sh
if [ "$1" = --version ]; then
  case "$0" in *codex) echo 'codex-cli 0.156.1';; *) echo '2.1.285 (Claude Code)';; esac
  exit 0
fi
dir=${CLAUDE_CONFIG_DIR:-"$HOME/.claude"}
if [ -n "${CODEX_HOME-}" ]; then dir=$CODEX_HOME; fi
if [ "$1" = --settings ] && [ "$3" = auth ] && [ "$4" = status ]; then
  if [ -f "$dir/.hidden-stored-login" ]; then echo '{"loggedIn":true,"authMethod":"claude.ai","apiKeySource":"none"}'; else echo '{"loggedIn":false}'; fi
  exit 0
fi
mkdir -p "$dir"
printf '%s\n%s\n%s\n%s\n%s\n%s\n%s\n' "$HOME" "$dir" "${ANTHROPIC_AUTH_TOKEN-unset}" "${ANTHROPIC_BASE_URL-unset}" "${CLAUDE_CODE_USE_BEDROCK-unset}" "${MCP_A-unset}" "${MCP_B-unset}" > "$dir/.launch-record"
printf '%s\n' "$*" > "$dir/.launch-args"
if [ "$1" = auth ] && [ "$2" = logout ]; then
  if [ -n "${CLAUDE_CONFIG_DIR-}" ]; then rm -f "$dir/.claude.json"; else rm -f "$HOME/.claude.json"; fi
fi
if [ -f "$dir/.hold" ]; then touch "$dir/.ready"; while [ -f "$dir/.hold" ]; do sleep .05; done; fi
if [ "$1" = auth ] && [ "$2" = status ]; then echo '{"loggedIn":false}'; fi
"#;

struct Fixture {
    root: PathBuf,
    ctx: Context,
    bindings: RefCell<BTreeMap<String, String>>,
}
impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "am-account-launch-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("home/.local/bin")).unwrap();
        std::fs::create_dir_all(root.join("project")).unwrap();
        let root = dunce::canonicalize(root).unwrap();
        let ctx = Context {
            home_dir: root.join("home"),
            data_dir: root.join("data"),
        };
        for bin in ["claude", "codex"] {
            let path = ctx.home_dir.join(".local/bin").join(bin);
            std::fs::write(&path, FAKE).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        std::fs::write(
            ctx.home_dir.join(".claude.json"),
            b"{\"oauthAccount\":{\"emailAddress\":\"main@example.invalid\"}}",
        )
        .unwrap();
        std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
        Self {
            root,
            ctx,
            bindings: RefCell::new(BTreeMap::new()),
        }
    }
    fn call(&self, command: &str, args: &[&str]) -> agent_manager_engine::Envelope {
        execute(
            &self.ctx,
            &Request {
                command: command.into(),
                args: args.iter().map(|v| (*v).into()).collect(),
                run_id: "account-launch-test".into(),
            },
        )
    }
    fn add(&self, id: &str, api: bool) {
        let env = if api {
            json!({"ANTHROPIC_AUTH_TOKEN": format!("${{TOKEN_{}}}", id.to_uppercase()), "ANTHROPIC_BASE_URL": format!("https://{}.example.invalid",id)})
        } else {
            json!({})
        };
        let def = json!({"profile":{"base_agent":"claude-code", "config_dir":format!("~/.claude-{id}"), "env":env}}).to_string();
        let out = self.call(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                &format!("agents.definitions.{id}"),
                "--value",
                &def,
            ],
        );
        assert!(out.ok, "{:?}", out.errors);
        let dir = self.ctx.home_dir.join(format!(".claude-{id}"));
        std::fs::create_dir_all(&dir).unwrap();
        let live = if api {
            json!({"env":{"ANTHROPIC_AUTH_TOKEN":format!("dummy-{id}-value"),"ANTHROPIC_BASE_URL":format!("https://{id}.example.invalid")}})
        } else {
            json!({})
        };
        std::fs::write(dir.join("settings.json"), live.to_string()).unwrap();
        let only = format!("profiles:{id}");
        self.bindings
            .borrow_mut()
            .insert(only.clone(), self.fetch_binding(&only));
    }
    fn secrets(&self) {
        let file = self.ctx.home_dir.join("secrets.env");
        std::fs::write(
            &file,
            "TOKEN_A=dummy-a-value\nTOKEN_B=dummy-b-value\nMCP_A=mcp-dummy-a\nMCP_B=mcp-dummy-b\n",
        )
        .unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            self.call("secrets:source", &["--path", file.to_str().unwrap()])
                .ok
        );
    }
    fn command(&self, only: &str, operation: &str, args: &[&str]) -> Command {
        let binding = self
            .bindings
            .borrow()
            .get(only)
            .cloned()
            .unwrap_or_else(|| self.fetch_binding(only));
        let mut cmd = Command::new(ENGINE);
        cmd.args([
            "--account-launch",
            "--home",
            self.ctx.home_dir.to_str().unwrap(),
            "--data-dir",
            self.ctx.data_dir.to_str().unwrap(),
            "--only",
            only,
            "--operation",
            operation,
            "--binding",
            &binding,
            "--",
        ])
        .args(args)
        .current_dir(self.root.join("project"))
        .env("HOME", self.root.join("wrong-home"))
        .env("CLAUDE_CONFIG_DIR", self.ctx.home_dir.join(".claude"))
        .env("CODEX_HOME", self.ctx.home_dir.join(".codex"))
        .env("ANTHROPIC_AUTH_TOKEN", "parent-dummy-value")
        .env("ANTHROPIC_BASE_URL", "https://parent.example.invalid")
        .env("CLAUDE_CODE_USE_BEDROCK", "1")
        .env("CLAUDE_CODE_OAUTH_TOKEN", "parent-oauth-dummy")
        .env("BASH_ENV", self.root.join("untrusted-shell"))
        .env("NODE_OPTIONS", "--require=untrusted-module");
        cmd
    }
    fn fetch_binding(&self, only: &str) -> String {
        let out = Command::new(ENGINE)
            .args(["accounts:command", "--json", "--only", only])
            .env("HOME", &self.ctx.home_dir)
            .env("AGENT_MANAGER_DATA_DIR", &self.ctx.data_dir)
            .current_dir(self.root.join("project"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        let envelope: Value = serde_json::from_slice(&out.stdout).unwrap();
        envelope["data"]["launch_value"]
            .as_str()
            .unwrap()
            .split(" --binding ")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .into()
    }
    fn run(&self, only: &str, op: &str, args: &[&str]) -> Output {
        self.command(only, op, args).output().unwrap()
    }
    fn record(&self, id: &str) -> Vec<String> {
        std::fs::read_to_string(
            self.ctx
                .home_dir
                .join(format!(".claude-{id}/.launch-record")),
        )
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn concurrent_accounts_use_distinct_contexts_and_leave_parent_main_and_other_account_unchanged() {
    let f = Fixture::new("parallel");
    f.secrets();
    f.add("a", true);
    f.add("b", true);
    let main = std::fs::read(f.ctx.home_dir.join(".claude.json")).unwrap();
    let a_settings = std::fs::read(f.ctx.home_dir.join(".claude-a/settings.json")).unwrap();
    let b_settings = std::fs::read(f.ctx.home_dir.join(".claude-b/settings.json")).unwrap();
    let parent = std::env::var_os("ANTHROPIC_AUTH_TOKEN");
    let mut a = f
        .command("profiles:a", "launch", &["--print", "hello"])
        .spawn()
        .unwrap();
    let mut b = f
        .command("profiles:b", "launch", &["--print", "hello"])
        .spawn()
        .unwrap();
    assert!(a.wait().unwrap().success());
    assert!(b.wait().unwrap().success());
    let a = f.record("a");
    let b = f.record("b");
    assert_eq!(a[0], f.ctx.home_dir.to_string_lossy());
    assert_eq!(b[0], a[0]);
    assert_ne!(a[1], b[1]);
    assert_eq!(a[2], "dummy-a-value");
    assert_eq!(b[2], "dummy-b-value");
    assert_eq!(a[3], "https://a.example.invalid");
    assert_eq!(b[3], "https://b.example.invalid");
    assert_eq!(a[4], "unset");
    assert_eq!(b[4], "unset");
    assert_eq!(std::env::var_os("ANTHROPIC_AUTH_TOKEN"), parent);
    assert_eq!(
        std::fs::read(f.ctx.home_dir.join(".claude.json")).unwrap(),
        main
    );
    assert_eq!(
        std::fs::read(f.ctx.home_dir.join(".claude-a/settings.json")).unwrap(),
        a_settings
    );
    assert_eq!(
        std::fs::read(f.ctx.home_dir.join(".claude-b/settings.json")).unwrap(),
        b_settings
    );
}

#[test]
fn main_logout_clears_external_directory_override_and_uses_home_dotfile() {
    let f = Fixture::new("main-logout");
    f.add("b", false);
    let b_auth = f.ctx.home_dir.join(".claude-b/.claude.json");
    std::fs::write(&b_auth, "{\"oauthAccount\":{}}").unwrap();
    let mut cmd = f.command("agents:claude-code", "logout", &[]);
    cmd.env("CLAUDE_CONFIG_DIR", f.ctx.home_dir.join(".claude-b"));
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!f.ctx.home_dir.join(".claude.json").exists());
    assert_eq!(std::fs::read(&b_auth).unwrap(), b"{\"oauthAccount\":{}}");
    let main = std::fs::read_to_string(f.ctx.home_dir.join(".claude/.launch-record")).unwrap();
    assert!(main
        .lines()
        .any(|line| line == f.ctx.home_dir.join(".claude").to_string_lossy()));
    assert!(!main.contains("parent-dummy-value"));
}

#[test]
fn identity_options_and_unknown_control_arguments_fail_before_cli_or_write() {
    let f = Fixture::new("args");
    f.add("a", false);
    for args in [
        vec!["-c", "model_provider=other"],
        vec!["--config", "auth=value"],
        vec!["--settings", "{}"],
        vec!["--profile", "other"],
        vec!["-pother"],
        vec!["--env", "OPENAI_API_KEY=x"],
        vec!["--add-dir", "/tmp"],
        vec!["--", "--settings", "{}"],
    ] {
        let out = f.run("profiles:a", "launch", &args);
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("CLI 参数"),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!f.ctx.home_dir.join(".claude-a/.launch-record").exists());
    }
    let mut cmd = f.command("profiles:a", "launch", &[]);
    cmd.arg("--home").arg(f.ctx.home_dir.to_str().unwrap());
    assert!(!cmd.output().unwrap().status.success());
}

#[test]
fn deleted_registration_and_manual_api_login_fail_closed() {
    let f = Fixture::new("deleted");
    f.secrets();
    f.add("a", true);
    let auth = f.ctx.home_dir.join(".claude-a/.claude.json");
    std::fs::write(
        &auth,
        "{\"oauthAccount\":{\"emailAddress\":\"other@example.invalid\"}}",
    )
    .unwrap();
    assert!(!f.run("profiles:a", "launch", &[]).status.success());
    assert!(!f.ctx.home_dir.join(".claude-a/.launch-record").exists());
    std::fs::remove_file(auth).unwrap();
    assert!(
        f.call(
            "policy:delete",
            &["--layer", "local", "--key", "agents.definitions.a"]
        )
        .ok
    );
    assert!(!f.run("profiles:a", "launch", &[]).status.success());
    assert!(!f.ctx.home_dir.join(".claude-a/.launch-record").exists());
}

#[test]
fn project_and_managed_identity_layers_and_broken_files_fail_closed() {
    let f = Fixture::new("project");
    f.add("a", false);
    let dir = f.root.join("project/.claude");
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("settings.local.json");
    for bytes in [
        b"{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://other.example.invalid\"}}".to_vec(),
        b"{\"apiKeyHelper\":\"other\"}".to_vec(),
        b"invalid-json".to_vec(),
        vec![b' '; 1024 * 1024 + 1],
    ] {
        std::fs::write(&path, bytes).unwrap();
        assert!(!f.run("profiles:a", "launch", &[]).status.success());
        assert!(!f.ctx.home_dir.join(".claude-a/.launch-record").exists());
    }
    std::fs::remove_file(path).unwrap();
    let managed = f.ctx.home_dir.join(".claude-a/managed-settings.json");
    std::fs::write(managed, "{\"policyHelper\":\"external\"}").unwrap();
    assert!(!f.run("profiles:a", "launch", &[]).status.success());
    assert!(!f.ctx.home_dir.join(".claude-a/.launch-record").exists());
}

#[test]
fn empty_hooks_work_and_active_hooks_block_launch_while_auth_uses_a_neutral_directory() {
    let f = Fixture::new("hooks");
    f.add("a", false);
    let dir = f.ctx.home_dir.join(".claude-a");
    let project = f.root.join("project/.claude");
    std::fs::create_dir(&project).unwrap();
    let paths = [
        dir.join("settings.json"),
        project.join("settings.local.json"),
        dir.join("managed-settings.json"),
    ];
    let empty = r#"{"hooks":{"SessionStart":[{},[]]}}"#;
    for path in &paths {
        std::fs::write(path, empty).unwrap();
    }
    let out = f.run("profiles:a", "launch", &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let main_before = std::fs::read(f.ctx.home_dir.join(".claude.json")).unwrap();
    for path in &paths {
        std::fs::remove_file(dir.join(".launch-record")).unwrap();
        std::fs::write(path, r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"overwrite-auth"}]}]}}"#).unwrap();
        let out = f.run("profiles:a", "launch", &[]);
        assert!(!out.status.success());
        let reason = String::from_utf8_lossy(&out.stderr);
        assert!(
            reason.contains("hooks") || reason.contains("项目或托管 settings"),
            "{reason}"
        );
        assert!(!dir.join(".launch-record").exists());
        if path == &paths[1] {
            let out = f.run("profiles:a", "status", &[]);
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        std::fs::write(path, empty).unwrap();
        let out = f.run("profiles:a", "launch", &[]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    assert_eq!(
        std::fs::read(f.ctx.home_dir.join(".claude.json")).unwrap(),
        main_before
    );
}

#[test]
fn parent_values_cannot_supply_account_credentials_without_an_explicit_source() {
    let f = Fixture::new("no-source");
    f.secrets();
    f.add("a", true);
    let dir = f.ctx.home_dir.join(".claude-a");
    let before = std::fs::read(dir.join("settings.json")).unwrap();
    assert!(f.call("secrets:source", &["--path", ""]).ok);
    let out = f
        .command("profiles:a", "launch", &[])
        .env("TOKEN_A", "dummy-a-value")
        .env("MCP_A", "mcp-dummy-a")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("显式选择"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!dir.join(".launch-record").exists());
    assert_eq!(std::fs::read(dir.join("settings.json")).unwrap(), before);
}

#[test]
fn bash_and_zsh_existing_cli_aliases_cannot_redirect_controlled_launcher() {
    let f = Fixture::new("shell-alias");
    f.add("a", false);
    let mut generate = Command::new(ENGINE);
    let out = generate
        .args(["accounts:command", "--json", "--only", "profiles:a"])
        .env("HOME", &f.ctx.home_dir)
        .env("AGENT_MANAGER_DATA_DIR", &f.ctx.data_dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    let launcher = value["data"]["launch_value"].as_str().unwrap();
    let file = f.root.join("aliases.sh");
    std::fs::write(
        &file,
        format!("alias claude='exit 89'\nalias work='{launcher}'\nwork --print hello\n"),
    )
    .unwrap();
    for shell in ["/bin/bash", "/bin/zsh"] {
        if !Path::new(shell).exists() {
            continue;
        }
        let mut cmd = Command::new(shell);
        if shell.ends_with("bash") {
            cmd.args(["--noprofile", "--norc", "-O", "expand_aliases"]);
        } else {
            cmd.arg("-f");
        }
        let out = cmd
            .arg(&file)
            .current_dir(f.root.join("project"))
            .env("HOME", f.root.join("wrong-home"))
            .env("CLAUDE_CONFIG_DIR", f.ctx.home_dir.join(".claude"))
            .env("ANTHROPIC_AUTH_TOKEN", "parent-dummy")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(f.record("a")[2], "unset");
        assert!(!f.ctx.home_dir.join(".claude/.launch-record").exists());
    }
}

#[test]
fn active_session_allows_other_accounts_and_blocks_auth_secret_and_registration_changes() {
    let f = Fixture::new("locks");
    f.secrets();
    f.add("a", true);
    f.add("b", true);
    let dir = f.ctx.home_dir.join(".claude-a");
    std::fs::write(dir.join(".hold"), "hold").unwrap();
    let mut a = f.command("profiles:a", "launch", &[]).spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !dir.join(".ready").exists() {
        assert!(
            a.try_wait().unwrap().is_none(),
            "fake CLI exited before ready"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "fake CLI did not start"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(f.run("profiles:b", "launch", &[]).status.success());
    assert!(!f.run("profiles:a", "logout", &[]).status.success());
    assert!(
        !f.call(
            "secrets:set",
            &["--var", "TOKEN_A", "--value", "replacement-dummy"]
        )
        .ok
    );
    assert!(
        !f.call(
            "policy:delete",
            &["--layer", "local", "--key", "agents.definitions.a"]
        )
        .ok
    );
    std::fs::remove_file(dir.join(".hold")).unwrap();
    assert!(a.wait().unwrap().success());
    assert!(
        f.call(
            "policy:delete",
            &["--layer", "local", "--key", "agents.definitions.a"]
        )
        .ok
    );
}

#[test]
fn old_binding_cannot_rebind_after_same_id_is_deleted_and_registered_again() {
    let f = Fixture::new("incarnation");
    f.add("a", false);
    let mut old = f.command("profiles:a", "launch", &[]);
    assert!(
        f.call(
            "policy:delete",
            &["--layer", "local", "--key", "agents.definitions.a"]
        )
        .ok
    );
    f.add("a", false);
    let out = old.output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("身份绑定已失效"));
    assert!(!f.ctx.home_dir.join(".claude-a/.launch-record").exists());
    assert!(f.run("profiles:a", "launch", &[]).status.success());
}

#[test]
fn own_mcp_references_are_injected_and_cross_account_or_missing_references_fail_closed() {
    let f = Fixture::new("mcp-runtime");
    f.secrets();
    f.add("a", false);
    f.add("b", false);
    let a = f.ctx.home_dir.join(".claude-a/.claude.json");
    let b = f.ctx.home_dir.join(".claude-b/.claude.json");
    for (file, name) in [(&a, "MCP_A"), (&b, "MCP_B")] {
        std::fs::write(file,json!({"mcpServers":{"owned":{"type":"http","url":"https://mcp.example.invalid","headers":{"Authorization":format!("${{{name}}}")}}}}).to_string()).unwrap();
    }
    assert!(f.run("profiles:a", "launch", &[]).status.success());
    assert!(f.run("profiles:b", "launch", &[]).status.success());
    assert_eq!(&f.record("a")[5..], &["mcp-dummy-a", "unset"]);
    assert_eq!(&f.record("b")[5..], &["unset", "mcp-dummy-b"]);
    std::fs::remove_file(f.ctx.home_dir.join(".claude-a/.launch-record")).unwrap();
    std::fs::write(&b, std::fs::read(&a).unwrap()).unwrap();
    let out = f.run("profiles:a", "launch", &[]);
    assert!(!out.status.success());
    let reason = String::from_utf8_lossy(&out.stderr);
    assert!(
        reason.contains("密钥变量被另一账号") || reason.contains("密钥变量被另一个账号"),
        "{reason}"
    );
    assert!(!f.ctx.home_dir.join(".claude-a/.launch-record").exists());
    std::fs::write(&b, "{}").unwrap();
    std::fs::write(&a,json!({"mcpServers":{"owned":{"type":"http","url":"https://mcp.example.invalid","headers":{"Authorization":"${MCP_MISSING}"}}}}).to_string()).unwrap();
    let out = f.run("profiles:a", "launch", &[]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("缺少引用"));
    assert!(!f.ctx.home_dir.join(".claude-a/.launch-record").exists());
}

#[test]
fn unsupported_cli_version_cannot_run_with_account_credentials() {
    let f = Fixture::new("version");
    f.add("a", false);
    std::fs::write(
        f.ctx.home_dir.join(".local/bin/claude"),
        FAKE.replace("2.1.285 (Claude Code)", "9.9.9 (Claude Code)"),
    )
    .unwrap();
    let out = f.run("profiles:a", "launch", &[]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("版本未纳入"));
    assert!(!f.ctx.home_dir.join(".claude-a/.launch-record").exists());
}

#[test]
fn draft_alias_can_be_previewed_but_cannot_be_written_or_executed() {
    let f = Fixture::new("draft");
    let definition =
        json!({"profile":{"base_agent":"claude-code","config_dir":"~/.claude-draft"}}).to_string();
    let result = f.call(
        "accounts:command",
        &["--only", "profiles:draft", "--definition", &definition],
    );
    assert!(result.ok, "{:?}", result.errors);
    let command = result.data["launch_value"].as_str().unwrap();
    assert!(command.contains("--binding preview-only"));
    let args = [
        "--bashrc-base",
        "missing",
        "--set",
        "draft",
        "--value",
        command,
    ];
    let preview = f.call("aliases:preview", &args);
    assert!(preview.ok, "{:?}", preview.errors);
    assert!(!f.ctx.data_dir.exists());
    assert!(!f.ctx.home_dir.join(".bashrc").exists());
    assert!(!f.call("aliases:apply", &args).ok);
    assert!(!f.ctx.home_dir.join(".bashrc").exists());
    f.add("draft", false);
    let out = Command::new(ENGINE)
        .args([
            "--account-launch",
            "--home",
            f.ctx.home_dir.to_str().unwrap(),
            "--data-dir",
            f.ctx.data_dir.to_str().unwrap(),
            "--only",
            "profiles:draft",
            "--operation",
            "launch",
            "--binding",
            "preview-only",
            "--",
        ])
        .current_dir(f.root.join("project"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("身份绑定已失效"));
    assert!(!f.ctx.home_dir.join(".claude-draft/.launch-record").exists());
}

#[test]
fn trusted_home_alias_generates_a_canonical_binding_that_runs_consistently() {
    let f = Fixture::new("trusted-home");
    f.add("a", false);
    let alias = f.root.join("home-alias");
    std::os::unix::fs::symlink(&f.ctx.home_dir, &alias).unwrap();
    let out = Command::new(ENGINE)
        .args(["accounts:command", "--json", "--only", "profiles:a"])
        .env("HOME", &alias)
        .env("AGENT_MANAGER_DATA_DIR", &f.ctx.data_dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap();
    let command = envelope["data"]["launch_value"].as_str().unwrap();
    assert!(!command.contains(alias.to_str().unwrap()));
    let out = Command::new("/bin/sh")
        .args(["-c", command])
        .current_dir(f.root.join("project"))
        .env("HOME", f.root.join("wrong-home"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(f.record("a")[0], f.ctx.home_dir.to_string_lossy());
}

#[test]
fn api_settings_cannot_hide_a_second_login_in_the_account_credential_store() {
    let f = Fixture::new("hidden-login");
    f.secrets();
    f.add("a", true);
    std::fs::write(
        f.ctx.home_dir.join(".claude-a/.hidden-stored-login"),
        "fake hidden slot",
    )
    .unwrap();
    let out = f.run("profiles:a", "launch", &[]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("凭据存储中存在"));
    assert!(!f.ctx.home_dir.join(".claude-a/.launch-record").exists());
    let status = f.call("agents:auth-status", &["--only", "profiles:a", "--probe"]);
    assert!(status.ok);
    assert_eq!(status.data["mixed"], true);
}

#[test]
fn unknown_stored_login_probe_output_never_establishes_an_empty_credential_store() {
    let f = Fixture::new("unknown-store");
    f.secrets();
    f.add("a", true);
    for result in ["{}", r#"{"loggedIn":true,"authMethod":"unrecognized"}"#] {
        std::fs::write(
            f.ctx.home_dir.join(".local/bin/claude"),
            FAKE.replace(r#"{"loggedIn":false}"#, result),
        )
        .unwrap();
        let out = f.run("profiles:a", "launch", &[]);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("无法确认账号凭据存储"));
        assert!(!f.ctx.home_dir.join(".claude-a/.launch-record").exists());
    }
}

#[test]
fn official_auth_actions_and_shared_sessions_use_the_same_explicit_context() {
    let f = Fixture::new("official");
    f.add("a", false);
    for action in ["login", "status", "logout", "launch"] {
        let out = f.run("profiles:a", action, &[]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(f.record("a")[2], "unset");
        assert_eq!(f.record("a")[3], "unset");
    }
    let dir = f.ctx.home_dir.join(".claude-a");
    std::fs::write(dir.join(".hold"), "hold").unwrap();
    let mut one = f.command("profiles:a", "launch", &[]).spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !dir.join(".ready").exists() {
        assert!(one.try_wait().unwrap().is_none());
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let mut two = f.command("profiles:a", "launch", &[]).spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(250));
    assert!(two.try_wait().unwrap().is_none());
    let out = f.run("profiles:a", "logout", &[]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("正在运行的会话"));
    std::fs::remove_file(dir.join(".hold")).unwrap();
    assert!(one.wait().unwrap().success());
    assert!(two.wait().unwrap().success());
    assert!(f.run("profiles:a", "logout", &[]).status.success());
}
