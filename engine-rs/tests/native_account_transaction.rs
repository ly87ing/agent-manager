//! Real account writes with isolated HOME, private fake credentials, and fake CLIs.
#[cfg(windows)]
mod common;

use agent_manager_engine::{execute, store, Context, Envelope, Request};
use serde_json::{json, Value};
use std::path::PathBuf;

struct Fixture {
    root: PathBuf,
    ctx: Context,
    source: PathBuf,
}
impl Fixture {
    fn new(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("am-account-txn-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        #[cfg(windows)]
        common::private_fixture_root(&root);
        let ctx = Context {
            home_dir: root.join("home"),
            data_dir: root.join("data"),
        };
        let bin = ctx.home_dir.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for (name, body) in [
                ("claude", "#!/bin/sh\nif [ \"$1\" = --version ]; then echo '2.1.285 (Claude Code)'; exit 0; fi\n[ \"$1\" = --settings ] && shift 2\n[ \"$1\" = auth ] && [ \"$2\" = status ] || exit 64\necho '{\"loggedIn\":false,\"authMethod\":\"none\"}'\nexit 1\n"),
                ("codex", "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'codex-cli 0.156.1'; exit 0; fi\n[ \"$1\" = login ] && [ \"$2\" = status ] || exit 64\necho 'Not logged in' >&2\nexit 1\n"),
            ] {
                let path = bin.join(name);
                std::fs::write(&path, body).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        #[cfg(windows)]
        common::install_fake_account_clis(&ctx.home_dir);
        let source = ctx.home_dir.join("private/account.env");
        let selected = run(
            &ctx,
            "secrets:source",
            &[
                "--path".into(),
                source.display().to_string(),
                "--create".into(),
            ],
        );
        assert!(selected.ok, "{:?}", selected.errors);
        Self { root, ctx, source }
    }
    fn config(&self, id: &str) -> PathBuf {
        self.ctx
            .home_dir
            .join(format!(".claude-{id}/settings.json"))
    }
    fn policy(&self) -> Value {
        store::effective_policy(&self.ctx).unwrap()
    }
    fn source_bytes(&self) -> Vec<u8> {
        std::fs::read(&self.source).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn run(ctx: &Context, command: &str, args: &[String]) -> Envelope {
    execute(
        ctx,
        &Request {
            command: command.into(),
            args: args.to_vec(),
            run_id: "account-transaction-test".into(),
        },
    )
}
fn def(id: &str, var: &str) -> Value {
    json!({"bin":"claude", "profile":{"base_agent":"claude-code", "kind":"claude",
        "config_dir":format!("~/.claude-{id}"), "env":{
            "ANTHROPIC_BASE_URL":format!("https://{id}.example.invalid"),
            "ANTHROPIC_AUTH_TOKEN":format!("${{{var}}}") }}})
}
fn draft_base(ctx: &Context, id: &str, definition: &Value) -> String {
    let preview = run(
        ctx,
        "profiles:preview",
        &[
            "--only".into(),
            format!("profiles:{id}"),
            "--definition".into(),
            definition.to_string(),
            "--on-save".into(),
        ],
    );
    assert!(preview.ok, "{:?}", preview.errors);
    preview.data["base"].as_str().unwrap().to_string()
}
fn create(ctx: &Context, id: &str, definition: &Value, var: &str, value: &str) -> Envelope {
    let base = draft_base(ctx, id, definition);
    create_at(ctx, id, definition, &base, var, value)
}
fn create_at(
    ctx: &Context,
    id: &str,
    definition: &Value,
    base: &str,
    var: &str,
    value: &str,
) -> Envelope {
    run(
        ctx,
        "profiles:create",
        &[
            "--only".into(),
            format!("profiles:{id}"),
            "--definition".into(),
            definition.to_string(),
            "--base".into(),
            base.into(),
            "--secret-var".into(),
            var.into(),
            "--secret-value".into(),
            value.into(),
        ],
    )
}
fn update(ctx: &Context, id: &str, var: &str, value: &str) -> Envelope {
    let status = run(ctx, "profiles:status", &[]);
    assert!(status.ok);
    let row = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == format!("profiles:{id}"))
        .unwrap();
    let candidate = json!({"env":{"ANTHROPIC_BASE_URL":format!("https://{id}.example.invalid/rotated"),
        "ANTHROPIC_AUTH_TOKEN":format!("${{{var}}}")}});
    run(
        ctx,
        "profiles:apply-definition",
        &[
            "--only".into(),
            format!("profiles:{id}"),
            "--base".into(),
            row["observed"]["base"].as_str().unwrap().into(),
            "--definition-base".into(),
            row["observed"]["definition_base"].as_str().unwrap().into(),
            "--candidate".into(),
            candidate.to_string(),
            "--secret-var".into(),
            var.into(),
            "--secret-value".into(),
            value.into(),
        ],
    )
}
fn reject_audit(ctx: &Context) {
    store::open_write(ctx).unwrap().execute_batch("CREATE TRIGGER reject_account_audit BEFORE INSERT ON operation_logs BEGIN SELECT RAISE(FAIL, 'fixture audit failure'); END;").unwrap();
}

#[test]
fn normal_create_and_rotation_keep_main_and_other_account_unchanged() {
    let f = Fixture::new("normal-ab");
    std::fs::create_dir_all(f.ctx.home_dir.join(".claude")).unwrap();
    let main = f.ctx.home_dir.join(".claude/settings.json");
    let main_auth = f.ctx.home_dir.join(".claude.json");
    std::fs::write(&main, b"{\"theme\":\"main-preserve\"}").unwrap();
    std::fs::write(
        &main_auth,
        b"{\"oauthAccount\":{\"emailAddress\":\"main@example.invalid\"}}",
    )
    .unwrap();
    assert!(
        create(
            &f.ctx,
            "a",
            &def("a", "TXN_A_KEY"),
            "TXN_A_KEY",
            "fixture-a-key"
        )
        .ok
    );
    assert!(
        create(
            &f.ctx,
            "b",
            &def("b", "TXN_B_KEY"),
            "TXN_B_KEY",
            "fixture-b-key"
        )
        .ok
    );
    let b = std::fs::read(f.config("b")).unwrap();
    let generation = f.policy()["agents"]["definitions"]["a"]["account_generation"].clone();
    assert!(generation.as_str().is_some_and(|value| !value.is_empty()));
    let result = update(&f.ctx, "a", "TXN_A_KEY", "fixture-a-rotated");
    assert!(result.ok, "{:?}", result.errors);
    let a: Value = serde_json::from_slice(&std::fs::read(f.config("a")).unwrap()).unwrap();
    assert_eq!(a["env"]["ANTHROPIC_AUTH_TOKEN"], "fixture-a-rotated");
    assert_eq!(std::fs::read(f.config("b")).unwrap(), b);
    assert_eq!(
        f.policy()["agents"]["definitions"]["a"]["account_generation"],
        generation
    );
    assert_eq!(
        std::fs::read(&main).unwrap(),
        b"{\"theme\":\"main-preserve\"}"
    );
    assert_eq!(
        std::fs::read(&main_auth).unwrap(),
        b"{\"oauthAccount\":{\"emailAddress\":\"main@example.invalid\"}}"
    );
    let policy = f.policy().to_string();
    assert!(
        !policy.contains("fixture-a-key")
            && !policy.contains("fixture-a-rotated")
            && !policy.contains("fixture-b-key")
    );
    let logs: Vec<String> = store::open_readonly(&f.ctx)
        .unwrap()
        .unwrap()
        .prepare("SELECT coalesce(detail_json,'') FROM operation_logs")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(logs
        .iter()
        .all(|log| !log.contains("fixture-a") && !log.contains("fixture-b")));
}

#[test]
fn failed_create_rolls_back_secret_config_registration_and_new_directory() {
    let f = Fixture::new("failed-create");
    let before_source = f.source_bytes();
    let before_policy = f.policy();
    reject_audit(&f.ctx);
    let result = create(
        &f.ctx,
        "a",
        &def("a", "TXN_CREATE_KEY"),
        "TXN_CREATE_KEY",
        "fixture-new-key",
    );
    assert!(!result.ok);
    assert_eq!(f.source_bytes(), before_source);
    assert_eq!(f.policy(), before_policy);
    assert!(!f.config("a").parent().unwrap().exists());
    let conn = store::open_readonly(&f.ctx).unwrap().unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='profile_bindings'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "rolled-back binding schema must not persist");
}

#[test]
fn failed_rotation_restores_source_config_and_definition_exactly() {
    let f = Fixture::new("failed-update");
    assert!(
        create(
            &f.ctx,
            "a",
            &def("a", "TXN_ROTATE_KEY"),
            "TXN_ROTATE_KEY",
            "fixture-old-key"
        )
        .ok
    );
    let before_source = f.source_bytes();
    let before_config = std::fs::read(f.config("a")).unwrap();
    let before_policy = f.policy();
    reject_audit(&f.ctx);
    let result = update(&f.ctx, "a", "TXN_ROTATE_KEY", "fixture-next-key");
    assert!(!result.ok);
    assert_eq!(f.source_bytes(), before_source);
    assert_eq!(std::fs::read(f.config("a")).unwrap(), before_config);
    assert_eq!(f.policy(), before_policy);
}

#[test]
fn stale_config_registration_directory_and_secret_conflicts_make_zero_writes() {
    let f = Fixture::new("conflicts");
    assert!(
        create(
            &f.ctx,
            "a",
            &def("a", "TXN_OWNER_KEY"),
            "TXN_OWNER_KEY",
            "fixture-owner-key"
        )
        .ok
    );
    let before_source = f.source_bytes();
    let before_policy = f.policy();
    let a = std::fs::read(f.config("a")).unwrap();
    let b_def = def("b", "TXN_B_STALE_KEY");
    let stale_base = draft_base(&f.ctx, "b", &b_def);
    std::fs::create_dir_all(f.config("b").parent().unwrap()).unwrap();
    std::fs::write(f.config("b"), b"{\"model\":\"external\"}").unwrap();
    assert!(
        !create_at(
            &f.ctx,
            "b",
            &b_def,
            &stale_base,
            "TXN_B_STALE_KEY",
            "fixture-stale-key"
        )
        .ok
    );
    assert!(
        !create_at(
            &f.ctx,
            "a",
            &def("a", "TXN_OWNER_KEY"),
            "missing",
            "TXN_OWNER_KEY",
            "fixture-conflict-key"
        )
        .ok
    );
    let shared_var = def("c", "TXN_OWNER_KEY");
    assert!(
        !create(
            &f.ctx,
            "c",
            &shared_var,
            "TXN_OWNER_KEY",
            "fixture-conflict-key"
        )
        .ok
    );
    let mut shared_dir = def("d", "TXN_D_KEY");
    shared_dir["profile"]["config_dir"] = "~/.claude-a".into();
    assert!(
        !create_at(
            &f.ctx,
            "d",
            &shared_dir,
            "missing",
            "TXN_D_KEY",
            "fixture-conflict-key"
        )
        .ok
    );
    assert_eq!(f.source_bytes(), before_source);
    assert_eq!(f.policy(), before_policy);
    assert_eq!(std::fs::read(f.config("a")).unwrap(), a);
    assert_eq!(
        std::fs::read(f.config("b")).unwrap(),
        b"{\"model\":\"external\"}"
    );
}

#[test]
fn existing_mixed_config_or_login_is_rejected_before_secret_and_registration() {
    for (tag, config, auth) in [
        (
            "mixed-token",
            br#"{"env":{"CLAUDE_CODE_OAUTH_TOKEN":"fixture-other-login"}}"#.as_slice(),
            None,
        ),
        (
            "mixed-provider",
            br#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1"}}"#.as_slice(),
            None,
        ),
        (
            "manual-login",
            b"{}".as_slice(),
            Some(br#"{"oauthAccount":{"emailAddress":"other@example.invalid"}}"#.as_slice()),
        ),
    ] {
        let f = Fixture::new(tag);
        std::fs::create_dir_all(f.config("a").parent().unwrap()).unwrap();
        std::fs::write(f.config("a"), config).unwrap();
        if let Some(auth) = auth {
            std::fs::write(f.config("a").parent().unwrap().join(".claude.json"), auth).unwrap();
        }
        let before_source = f.source_bytes();
        let before_policy = f.policy();
        let result = create(
            &f.ctx,
            "a",
            &def("a", "TXN_MIXED_KEY"),
            "TXN_MIXED_KEY",
            "fixture-account-key",
        );
        assert!(!result.ok, "{tag} must fail");
        assert_eq!(f.source_bytes(), before_source);
        assert_eq!(f.policy(), before_policy);
        assert_eq!(std::fs::read(f.config("a")).unwrap(), config);
    }
}

#[cfg(unix)]
#[test]
fn configured_source_failure_never_uses_parent_same_name_and_never_writes() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new("source-failure");
    let var = "TXN_SOURCE_FAILURE_KEY_UNIQUE";
    std::fs::write(&f.source, format!("{var}=\"fixture-source-a\"\n")).unwrap();
    std::fs::set_permissions(&f.source, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::env::set_var(var, "fixture-parent-b");
    let before = f.source_bytes();
    let policy = f.policy();
    let result = run(
        &f.ctx,
        "profiles:create",
        &[
            "--only".into(),
            "profiles:a".into(),
            "--definition".into(),
            def("a", var).to_string(),
            "--base".into(),
            "missing".into(),
        ],
    );
    assert!(!result.ok);
    assert_eq!(f.source_bytes(), before);
    assert_eq!(f.policy(), policy);
    assert!(!f.config("a").exists());
    assert_eq!(std::env::var(var).unwrap(), "fixture-parent-b");
    assert!(!format!("{:?}", result.errors).contains("fixture-parent-b"));
    std::env::remove_var(var);
}

#[test]
fn malformed_or_oversized_config_and_unconfigured_secret_source_fail_without_registration() {
    for (tag, bytes) in [
        ("malformed", b"{broken".to_vec()),
        ("oversized", vec![b' '; 1024 * 1024 + 1]),
    ] {
        let f = Fixture::new(tag);
        std::fs::create_dir_all(f.config("a").parent().unwrap()).unwrap();
        std::fs::write(f.config("a"), &bytes).unwrap();
        let source = f.source_bytes();
        let policy = f.policy();
        let result = create_at(
            &f.ctx,
            "a",
            &def("a", "TXN_INVALID_KEY"),
            "missing",
            "TXN_INVALID_KEY",
            "fixture-account-key",
        );
        assert!(!result.ok);
        assert_eq!(f.source_bytes(), source);
        assert_eq!(f.policy(), policy);
        assert_eq!(std::fs::read(f.config("a")).unwrap(), bytes);
    }
    let f = Fixture::new("unconfigured");
    assert!(run(&f.ctx, "secrets:source", &["--path".into(), "".into()]).ok);
    let policy = f.policy();
    let source = f.source_bytes();
    assert!(
        !create(
            &f.ctx,
            "a",
            &def("a", "TXN_UNSET_KEY"),
            "TXN_UNSET_KEY",
            "fixture-account-key"
        )
        .ok
    );
    assert_eq!(f.policy(), policy);
    assert_eq!(f.source_bytes(), source);
    assert!(!f.config("a").exists());
    std::env::set_var("TXN_ENV_ONLY_KEY", "fixture-parent-only-key");
    let inherited = run(
        &f.ctx,
        "profiles:create",
        &[
            "--only".into(),
            "profiles:a".into(),
            "--definition".into(),
            def("a", "TXN_ENV_ONLY_KEY").to_string(),
            "--base".into(),
            "missing".into(),
        ],
    );
    std::env::remove_var("TXN_ENV_ONLY_KEY");
    assert!(
        !inherited.ok,
        "an inherited variable cannot supply account credentials"
    );
    assert_eq!(f.policy(), policy);
    assert_eq!(f.source_bytes(), source);
    assert!(!f.config("a").exists());
}

#[test]
fn active_account_session_blocks_rotation_without_blocking_another_account() {
    use fs2::FileExt;
    let f = Fixture::new("active-account");
    assert!(
        create(
            &f.ctx,
            "a",
            &def("a", "TXN_ACTIVE_A"),
            "TXN_ACTIVE_A",
            "fixture-a-key"
        )
        .ok
    );
    let dir = std::fs::canonicalize(f.config("a").parent().unwrap()).unwrap();
    let name = agent_manager_engine::native_mcp::model::sha256_hex(&dir.to_string_lossy());
    // Simulate an existing old-version session; upgraded operations never create legacy files.
    std::fs::create_dir_all(f.ctx.data_dir.join("account-locks")).unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(f.ctx.data_dir.join("account-locks").join(name))
        .unwrap();
    FileExt::lock_shared(&lock).unwrap();
    let source = f.source_bytes();
    let config = std::fs::read(f.config("a")).unwrap();
    let policy = f.policy();
    let result = update(&f.ctx, "a", "TXN_ACTIVE_A", "fixture-refused-key");
    assert!(!result.ok);
    assert!(result.errors.iter().any(|err| err.message.contains("会话")));
    assert_eq!(f.source_bytes(), source);
    assert_eq!(f.policy(), policy);
    assert_eq!(std::fs::read(f.config("a")).unwrap(), config);
    let seeded = run(
        &f.ctx,
        "profiles:seed",
        &[
            "--only".into(),
            "profiles:a".into(),
            "--items".into(),
            "permissions".into(),
        ],
    );
    assert!(!seeded.ok, "seed must respect an active account lock");
    assert_eq!(f.source_bytes(), source);
    assert_eq!(std::fs::read(f.config("a")).unwrap(), config);
    let b = create(
        &f.ctx,
        "b",
        &def("b", "TXN_ACTIVE_B"),
        "TXN_ACTIVE_B",
        "fixture-b-key",
    );
    assert!(b.ok, "{:?}", b.errors);
    assert_eq!(std::fs::read(f.config("a")).unwrap(), config);
    drop(lock);
}

#[test]
fn concurrent_creates_serialize_shared_source_or_refuse_without_partial_changes() {
    let f = Fixture::new("concurrent-ab");
    let workers: Vec<_> = [
        ("a", "TXN_CONCURRENT_A", "fixture-concurrent-a"),
        ("b", "TXN_CONCURRENT_B", "fixture-concurrent-b"),
    ]
    .into_iter()
    .map(|(id, var, key)| {
        let ctx = Context {
            home_dir: f.ctx.home_dir.clone(),
            data_dir: f.ctx.data_dir.clone(),
        };
        (
            id,
            var,
            key,
            std::thread::spawn(move || create(&ctx, id, &def(id, var), var, key)),
        )
    })
    .collect();
    let results: Vec<_> = workers
        .into_iter()
        .map(|(id, var, key, worker)| (id, var, key, worker.join().unwrap()))
        .collect();
    for (id, var, key, result) in results {
        if !result.ok {
            assert!(
                result
                    .errors
                    .iter()
                    .any(|err| err.message.contains("正在执行") || err.message.contains("并发")),
                "{:?}",
                result.errors
            );
            assert!(!f.config(id).exists());
            assert!(!String::from_utf8(f.source_bytes()).unwrap().contains(var));
            let retry = create(&f.ctx, id, &def(id, var), var, key);
            assert!(retry.ok, "{:?}", retry.errors);
        }
    }
    for (id, key) in [("a", "fixture-concurrent-a"), ("b", "fixture-concurrent-b")] {
        let config: Value = serde_json::from_slice(&std::fs::read(f.config(id)).unwrap()).unwrap();
        assert_eq!(config["env"]["ANTHROPIC_AUTH_TOKEN"], key);
        assert_eq!(
            config["env"]["ANTHROPIC_BASE_URL"],
            format!("https://{id}.example.invalid")
        );
    }
}

#[cfg(unix)]
#[test]
fn source_change_after_plan_fails_cas_without_overwriting_external_bytes() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new("source-cas");
    std::fs::create_dir_all(f.config("a").parent().unwrap()).unwrap();
    let cli = f.ctx.home_dir.join(".local/bin/claude");
    std::fs::write(
        &cli,
        r#"#!/bin/sh
if [ "$1" = --version ]; then echo '2.1.285 (Claude Code)'; exit 0; fi
[ "$1" = --settings ] && shift 2
[ "$1" = auth ] && [ "$2" = status ] || exit 64
: > "$HOME/probe-started"
while [ ! -e "$HOME/probe-release" ]; do sleep 0.02; done
echo '{"loggedIn":false,"authMethod":"none"}'
exit 1
"#,
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let policy = f.policy();
    let ctx = Context {
        home_dir: f.ctx.home_dir.clone(),
        data_dir: f.ctx.data_dir.clone(),
    };
    let worker = std::thread::spawn(move || {
        create(
            &ctx,
            "a",
            &def("a", "TXN_SOURCE_CAS"),
            "TXN_SOURCE_CAS",
            "fixture-attempt-key",
        )
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !f.ctx.home_dir.join("probe-started").exists()
        && std::time::Instant::now() < deadline
        && !worker.is_finished()
    {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let reached_probe = f.ctx.home_dir.join("probe-started").exists();
    let external = b"TXN_SOURCE_CAS=\"fixture-external-source\"\n";
    if reached_probe {
        std::fs::write(&f.source, external).unwrap();
    }
    std::fs::write(f.ctx.home_dir.join("probe-release"), b"").unwrap();
    let result = worker.join().unwrap();
    assert!(
        reached_probe,
        "fixture must reach the real CLI probe: {:?}",
        result.errors
    );
    assert!(!result.ok);
    assert!(
        result
            .errors
            .iter()
            .any(|err| err.message.contains("外部密钥来源已变化，没有写入")),
        "must reject the planned source at commit: {:?}",
        result.errors
    );
    assert_eq!(f.source_bytes(), external);
    assert_eq!(f.policy(), policy);
    assert!(!f.config("a").exists());
}

#[test]
fn adopt_accepts_owned_live_identity_but_rejects_mixed_auth_and_active_session() {
    use fs2::FileExt;
    let f = Fixture::new("adopt-context");
    let definition = def("a", "TXN_ADOPT_KEY");
    assert!(
        create(
            &f.ctx,
            "a",
            &definition,
            "TXN_ADOPT_KEY",
            "fixture-adopt-key"
        )
        .ok
    );
    let mut live: Value = serde_json::from_slice(&std::fs::read(f.config("a")).unwrap()).unwrap();
    let generation = f.policy()["agents"]["definitions"]["a"]["account_generation"].clone();
    live["env"]["ANTHROPIC_BASE_URL"] = "https://live.example.invalid".into();
    std::fs::write(f.config("a"), live.to_string()).unwrap();
    let base = draft_base(&f.ctx, "a", &definition);
    let source = f.source_bytes();
    let config = std::fs::read(f.config("a")).unwrap();
    let adopted = run(
        &f.ctx,
        "profiles:adopt",
        &["--only".into(), "profiles:a".into(), "--base".into(), base],
    );
    assert!(adopted.ok, "{:?}", adopted.errors);
    assert_eq!(
        f.policy()["agents"]["definitions"]["a"]["profile"]["env"]["ANTHROPIC_BASE_URL"],
        "https://live.example.invalid"
    );
    assert_eq!(
        f.policy()["agents"]["definitions"]["a"]["account_generation"],
        generation
    );
    assert_eq!(f.source_bytes(), source);
    assert_eq!(std::fs::read(f.config("a")).unwrap(), config);

    live["env"]["CLAUDE_CODE_OAUTH_TOKEN"] = "fixture-foreign-token".into();
    std::fs::write(f.config("a"), live.to_string()).unwrap();
    let base = draft_base(&f.ctx, "a", &definition);
    let policy = f.policy();
    let mixed = run(
        &f.ctx,
        "profiles:adopt",
        &["--only".into(), "profiles:a".into(), "--base".into(), base],
    );
    assert!(!mixed.ok);
    assert_eq!(f.policy(), policy);
    assert_eq!(f.source_bytes(), source);
    assert_eq!(
        std::fs::read(f.config("a")).unwrap(),
        live.to_string().as_bytes()
    );

    live["env"]
        .as_object_mut()
        .unwrap()
        .remove("CLAUDE_CODE_OAUTH_TOKEN");
    std::fs::write(f.config("a"), live.to_string()).unwrap();
    let base = draft_base(&f.ctx, "a", &definition);
    let policy = f.policy();
    let dir = std::fs::canonicalize(f.config("a").parent().unwrap()).unwrap();
    let name = agent_manager_engine::native_mcp::model::sha256_hex(&dir.to_string_lossy());
    // Simulate an existing old-version session; upgraded operations never create legacy files.
    std::fs::create_dir_all(f.ctx.data_dir.join("account-locks")).unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(f.ctx.data_dir.join("account-locks").join(name))
        .unwrap();
    FileExt::lock_shared(&lock).unwrap();
    let active = run(
        &f.ctx,
        "profiles:adopt",
        &["--only".into(), "profiles:a".into(), "--base".into(), base],
    );
    assert!(!active.ok);
    assert!(active.errors.iter().any(|err| err.message.contains("会话")));
    assert_eq!(f.policy(), policy);
    assert_eq!(f.source_bytes(), source);
    assert_eq!(
        std::fs::read(f.config("a")).unwrap(),
        live.to_string().as_bytes()
    );
}

#[test]
fn recreating_account_uses_new_generation_and_variable_and_rejects_client_generation() {
    let f = Fixture::new("account-generation");
    let definition = def("a", "TXN_GENERATION_KEY");
    let created = create(
        &f.ctx,
        "a",
        &definition,
        "TXN_GENERATION_KEY",
        "fixture-generation-key",
    );
    assert!(created.ok, "{:?}", created.errors);
    let first = f.policy()["agents"]["definitions"]["a"]["account_generation"].clone();
    let removed = run(
        &f.ctx,
        "policy:set",
        &[
            "--layer".into(),
            "local".into(),
            "--key".into(),
            "agents.definitions.a".into(),
            "--value".into(),
            json!({"_delete":true}).to_string(),
        ],
    );
    assert!(removed.ok, "{:?}", removed.errors);
    let source = f.source_bytes();
    let policy = f.policy();
    let config = std::fs::read(f.config("a")).unwrap();
    let reused = create(
        &f.ctx,
        "a",
        &definition,
        "TXN_GENERATION_KEY",
        "fixture-generation-key",
    );
    assert!(
        !reused.ok,
        "a deleted account's credential name remains reserved"
    );
    assert_eq!(f.source_bytes(), source);
    assert_eq!(f.policy(), policy);
    assert_eq!(std::fs::read(f.config("a")).unwrap(), config);
    let recreated = create(
        &f.ctx,
        "a",
        &def("a", "TXN_GENERATION_NEW_KEY"),
        "TXN_GENERATION_NEW_KEY",
        "fixture-new-generation-key",
    );
    assert!(recreated.ok, "{:?}", recreated.errors);
    let second = f.policy()["agents"]["definitions"]["a"]["account_generation"].clone();
    assert_ne!(
        first, second,
        "recreation after deletion must have a fresh account generation"
    );
    let mut supplied = def("b", "TXN_GENERATION_B");
    supplied["account_generation"] = first;
    let source = f.source_bytes();
    let policy = f.policy();
    let refused = create(
        &f.ctx,
        "b",
        &supplied,
        "TXN_GENERATION_B",
        "fixture-generation-b",
    );
    assert!(!refused.ok);
    assert_eq!(f.policy(), policy);
    assert_eq!(f.source_bytes(), source);
    assert!(!f.config("b").exists());
}

#[test]
fn legacy_rotation_reserves_removed_and_new_reference_names_in_one_transaction() {
    let f = Fixture::new("legacy-ownership-rotation");
    let created = create(
        &f.ctx,
        "a",
        &def("a", "TXN_REMOVED_OLD_KEY"),
        "TXN_REMOVED_OLD_KEY",
        "fixture-old-owned-key",
    );
    assert!(created.ok, "{:?}", created.errors);
    // Reproduce a pre-ownership-table database using only this isolated fixture.
    store::open_write(&f.ctx)
        .unwrap()
        .execute("DELETE FROM secret_variable_owners", [])
        .unwrap();
    let rotated = update(&f.ctx, "a", "TXN_REMOVED_NEW_KEY", "fixture-new-owned-key");
    assert!(rotated.ok, "{:?}", rotated.errors);
    let conn = store::open_readonly(&f.ctx).unwrap().unwrap();
    for name in ["TXN_REMOVED_OLD_KEY", "TXN_REMOVED_NEW_KEY"] {
        let owner: String = conn
            .query_row(
                "SELECT account FROM secret_variable_owners WHERE name=?1",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(owner, "a");
    }
    drop(conn);
    let source = f.source_bytes();
    let policy = f.policy();
    let config = std::fs::read(f.config("a")).unwrap();
    let reused = create(
        &f.ctx,
        "b",
        &def("b", "TXN_REMOVED_OLD_KEY"),
        "TXN_REMOVED_OLD_KEY",
        "fixture-borrowed-key",
    );
    assert!(!reused.ok);
    assert_eq!(f.source_bytes(), source);
    assert_eq!(f.policy(), policy);
    assert_eq!(std::fs::read(f.config("a")).unwrap(), config);
    assert!(!f.config("b").exists());
}

#[test]
fn adopt_claims_reference_names_and_rolls_back_claims_with_failed_registration() {
    let f = Fixture::new("adopt-ownership-transaction");
    let definition = def("a", "TXN_ADOPT_OWNED_KEY");
    let created = create(
        &f.ctx,
        "a",
        &definition,
        "TXN_ADOPT_OWNED_KEY",
        "fixture-adopt-owned-key",
    );
    assert!(created.ok, "{:?}", created.errors);
    store::open_write(&f.ctx)
        .unwrap()
        .execute("DELETE FROM secret_variable_owners", [])
        .unwrap();
    let mut live: Value = serde_json::from_slice(&std::fs::read(f.config("a")).unwrap()).unwrap();
    live["env"]["ANTHROPIC_BASE_URL"] = "https://adopted-owned.example.invalid".into();
    std::fs::write(f.config("a"), live.to_string()).unwrap();
    let base = draft_base(&f.ctx, "a", &definition);
    let args = ["--only".into(), "profiles:a".into(), "--base".into(), base];
    let source = f.source_bytes();
    let policy = f.policy();
    let config = std::fs::read(f.config("a")).unwrap();
    reject_audit(&f.ctx);
    let refused = run(&f.ctx, "profiles:adopt", &args);
    assert!(!refused.ok);
    assert_eq!(f.source_bytes(), source);
    assert_eq!(f.policy(), policy);
    assert_eq!(std::fs::read(f.config("a")).unwrap(), config);
    let conn = store::open_write(&f.ctx).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM secret_variable_owners", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0, "failed adopt must roll back its reservation");
    conn.execute_batch("DROP TRIGGER reject_account_audit")
        .unwrap();
    drop(conn);
    let adopted = run(&f.ctx, "profiles:adopt", &args);
    assert!(adopted.ok, "{:?}", adopted.errors);
    let owner: String = store::open_readonly(&f.ctx)
        .unwrap()
        .unwrap()
        .query_row(
            "SELECT account FROM secret_variable_owners WHERE name='TXN_ADOPT_OWNED_KEY'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(owner, "a");
    assert_eq!(f.source_bytes(), source);
    assert_eq!(std::fs::read(f.config("a")).unwrap(), config);
}

#[test]
fn seed_and_shared_settings_preserve_owned_identity_and_refuse_mixed_or_active_accounts() {
    use fs2::FileExt;
    let f = Fixture::new("shared-context");
    let main = f.ctx.home_dir.join(".claude/settings.json");
    std::fs::create_dir_all(main.parent().unwrap()).unwrap();
    std::fs::write(&main, json!({"permissions":{"allow":["Read"]}}).to_string()).unwrap();
    for (id, var) in [("a", "TXN_SHARED_A"), ("b", "TXN_SHARED_B")] {
        let created = create(
            &f.ctx,
            id,
            &def(id, var),
            var,
            &format!("fixture-shared-{id}"),
        );
        assert!(created.ok, "{:?}", created.errors);
    }
    let source = f.source_bytes();
    let b = std::fs::read(f.config("b")).unwrap();
    let identity: Value = serde_json::from_slice::<Value>(&std::fs::read(f.config("a")).unwrap())
        .unwrap()["env"]
        .clone();
    let seeded = run(
        &f.ctx,
        "profiles:seed",
        &[
            "--only".into(),
            "profiles:a".into(),
            "--items".into(),
            "permissions".into(),
        ],
    );
    assert!(seeded.ok, "{:?}", seeded.errors);
    assert_eq!(seeded.data["results"][0]["state"], "copied");
    let seeded_config: Value =
        serde_json::from_slice(&std::fs::read(f.config("a")).unwrap()).unwrap();
    assert_eq!(seeded_config["env"], identity);
    assert_eq!(seeded_config["permissions"], json!({"allow":["Read"]}));

    std::fs::write(
        &main,
        json!({"permissions":{"allow":["Read", "Edit"]}}).to_string(),
    )
    .unwrap();
    let shared_status = run(
        &f.ctx,
        "profiles:shared-status",
        &["--only".into(), "profiles:a".into()],
    );
    assert!(shared_status.ok, "{:?}", shared_status.errors);
    let item = shared_status.data["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["item"] == "permissions")
        .unwrap();
    let shared_args = vec![
        "--only".into(),
        "profiles:a".into(),
        "--item".into(),
        "permissions".into(),
        "--source-base".into(),
        item["source_base"].as_str().unwrap().into(),
        "--target-base".into(),
        item["target_base"].as_str().unwrap().into(),
    ];
    let dir = std::fs::canonicalize(f.config("a").parent().unwrap()).unwrap();
    let name = agent_manager_engine::native_mcp::model::sha256_hex(&dir.to_string_lossy());
    // Simulate an existing old-version session; upgraded operations never create legacy files.
    std::fs::create_dir_all(f.ctx.data_dir.join("account-locks")).unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(f.ctx.data_dir.join("account-locks").join(name))
        .unwrap();
    FileExt::lock_shared(&lock).unwrap();
    let before = std::fs::read(f.config("a")).unwrap();
    let blocked = run(&f.ctx, "profiles:shared-apply", &shared_args);
    assert!(!blocked.ok);
    assert_eq!(std::fs::read(f.config("a")).unwrap(), before);
    drop(lock);
    let shared = run(&f.ctx, "profiles:shared-apply", &shared_args);
    assert!(shared.ok, "{:?}", shared.errors);
    let mut live: Value = serde_json::from_slice(&std::fs::read(f.config("a")).unwrap()).unwrap();
    assert_eq!(live["env"], identity);
    assert_eq!(live["permissions"], json!({"allow":["Read", "Edit"]}));

    live["env"]["CLAUDE_CODE_OAUTH_TOKEN"] = "fixture-foreign-token".into();
    std::fs::write(f.config("a"), live.to_string()).unwrap();
    let mixed = std::fs::read(f.config("a")).unwrap();
    let seed_refused = run(
        &f.ctx,
        "profiles:seed",
        &[
            "--only".into(),
            "profiles:a".into(),
            "--items".into(),
            "permissions".into(),
        ],
    );
    let shared_refused = run(&f.ctx, "profiles:shared-apply", &shared_args);
    assert!(!seed_refused.ok);
    assert!(!shared_refused.ok);
    assert_eq!(std::fs::read(f.config("a")).unwrap(), mixed);
    assert_eq!(std::fs::read(f.config("b")).unwrap(), b);
    assert_eq!(f.source_bytes(), source);
    assert_eq!(
        std::fs::read(&main).unwrap(),
        json!({"permissions":{"allow":["Read", "Edit"]}})
            .to_string()
            .as_bytes()
    );
}
