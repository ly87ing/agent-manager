use agent_manager_engine::{execute, store, Context, Request};
use fs2::FileExt;
use serde_json::json;
use std::fs::OpenOptions;
use std::path::PathBuf;

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    let base = std::env::temp_dir().join(format!("am-native-test-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&base);
    let ctx = Context {
        data_dir: base.join("data"),
        home_dir: base.join("home"),
    };
    (base, ctx)
}

fn request(command: &str, args: &[&str]) -> Request {
    Request {
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        run_id: "test-run-1".to_string(),
    }
}

fn set_agent(ctx: &Context, id: &str, bin: &str, install: &str) {
    let value = json!({
        "display_name": id,
        "bin": bin,
        "preferred_provider": "local-test",
        "providers": {"local-test": {"install": install}}
    });
    set_agent_definition(ctx, id, value);
}

fn set_agent_definition(ctx: &Context, id: &str, value: serde_json::Value) {
    let raw = serde_json::to_string(&value).unwrap();
    let env = execute(
        ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                &format!("agents.definitions.{id}"),
                "--value",
                &raw,
            ],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
}

#[test]
fn request_helpers_parse_flags_and_values() {
    let req = request(
        "rules:plan",
        &[
            "--only",
            "rules:a",
            "--only",
            "rules:b",
            "--dry-run",
            "--key",
            "k",
        ],
    );
    assert!(req.flag("--dry-run"));
    assert!(!req.flag("--offline"));
    assert_eq!(req.value("--key"), Some("k"));
    assert_eq!(req.value("--missing"), None);
    assert_eq!(req.values("--only"), vec!["rules:a", "rules:b"]);
}

#[test]
fn status_queries_never_create_db() {
    let (base, ctx) = temp_ctx("readonly");
    let env = execute(&ctx, &request("db:status", &[]));
    assert!(env.ok, "{:?}", env.errors);
    assert!(!store::db_path(&ctx).exists());
    assert!(!ctx.data_dir.exists());
    let env = execute(&ctx, &request("policy:get", &[]));
    assert!(env.ok);
    assert_eq!(env.data["layers"].as_object().unwrap().len(), 1);
    assert!(env.data["layers"]
        .as_object()
        .unwrap()
        .contains_key("sqlite"));
    assert!(env.data["layers"]["sqlite"].is_null());
    assert!(env.data["attribution"]
        .as_object()
        .unwrap()
        .values()
        .all(|value| value == "builtin"));
    assert!(!store::db_path(&ctx).exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn builtin_policy_is_default_effective() {
    let (base, ctx) = temp_ctx("builtin");
    let env = execute(&ctx, &request("policy:get", &[]));
    let effective = env.data["effective"].clone();
    assert_eq!(
        effective["agents"]["definitions"]["codex"]["providers"]["native"]["install"],
        "curl -fsSL https://chatgpt.com/codex/install.sh | sh"
    );
    assert_eq!(
        effective["agents"]["definitions"]["claude-code"]["providers"]["native"]["upgrade"],
        "claude update"
    );
    // 无证据的卸载命令不提供
    assert!(
        effective["agents"]["definitions"]["claude-code"]["providers"]["native"]
            .get("uninstall")
            .is_none()
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn first_set_initializes_db_and_freezes_builtin() {
    let (base, ctx) = temp_ctx("firstwrite");
    let env = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.codex.enabled",
                "--value",
                "false",
            ],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
    assert!(store::db_path(&ctx).exists());
    let effective = store::effective_policy(&ctx).unwrap();
    assert_eq!(effective["agents"]["codex"]["enabled"], false);
    // 内置默认已固化进库
    assert!(effective["agents"]["definitions"]["claude-code"].is_object());
    // 只读重开（等价重启进程）仍是新值
    let env = execute(&ctx, &request("policy:get", &[]));
    assert_eq!(env.data["effective"]["agents"]["codex"]["enabled"], false);
    assert_eq!(env.data["layers"].as_object().unwrap().len(), 1);
    assert!(env.data["layers"]["sqlite"].is_object());
    assert!(env.data["attribution"]
        .as_object()
        .unwrap()
        .values()
        .all(|value| value == "sqlite"));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn dry_run_set_writes_nothing() {
    let (base, ctx) = temp_ctx("dryrun");
    let env = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.codex.enabled",
                "--value",
                "false",
                "--dry-run",
            ],
        ),
    );
    assert!(env.ok);
    assert!(env.warnings.iter().any(|w| w.code == "DRY_RUN"));
    assert!(!store::db_path(&ctx).exists());
    assert!(!ctx.data_dir.exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn replace_semantics_and_delete_roundtrip() {
    let (base, ctx) = temp_ctx("replace");
    execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp",
                "--value",
                "{\"definitions\":{\"srv\":{\"command\":\"a\",\"keep\":1}}}",
            ],
        ),
    );
    // _replace 整条替换而不是深合并
    let env = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions",
                "--value",
                "{\"_replace\":true,\"srv\":{\"command\":\"b\"}}",
            ],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
    let effective = store::effective_policy(&ctx).unwrap();
    assert_eq!(effective["mcp"]["definitions"]["srv"]["command"], "b");
    assert!(effective["mcp"]["definitions"]["srv"].get("keep").is_none());
    // delete 存在的键成功、再删报错
    let env = execute(
        &ctx,
        &request(
            "policy:delete",
            &["--layer", "local", "--key", "mcp.definitions.srv"],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
    let env = execute(
        &ctx,
        &request(
            "policy:delete",
            &["--layer", "local", "--key", "mcp.definitions.srv"],
        ),
    );
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "POLICY_KEY_NOT_FOUND");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn delete_requires_local_layer_without_touching_store() {
    let (base, ctx) = temp_ctx("delete-layer");
    for args in [
        vec!["--key", "agents.missing"],
        vec!["--layer", "synced", "--key", "agents.missing"],
    ] {
        let env = execute(&ctx, &request("policy:delete", &args));
        assert!(!env.ok);
        assert_eq!(env.errors[0].code, "USAGE_ERROR");
    }
    assert!(!store::db_path(&ctx).exists());
    assert!(!ctx.data_dir.exists());

    let set = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.keep",
                "--value",
                "true",
            ],
        ),
    );
    assert!(set.ok, "{:?}", set.errors);
    let before = store::effective_policy(&ctx).unwrap();
    let count_before: i64 = store::open_readonly(&ctx)
        .unwrap()
        .unwrap()
        .query_row("SELECT count(*) FROM operation_logs", [], |row| row.get(0))
        .unwrap();
    for args in [
        vec!["--key", "agents.keep"],
        vec!["--layer", "synced", "--key", "agents.keep"],
    ] {
        let env = execute(&ctx, &request("policy:delete", &args));
        assert!(!env.ok);
        assert_eq!(env.errors[0].code, "USAGE_ERROR");
    }
    assert_eq!(store::effective_policy(&ctx).unwrap(), before);
    let count_after: i64 = store::open_readonly(&ctx)
        .unwrap()
        .unwrap()
        .query_row("SELECT count(*) FROM operation_logs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count_after, count_before);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn runs_list_and_get_reflect_audit_trail() {
    let (base, ctx) = temp_ctx("runs");
    execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.codex.enabled",
                "--value",
                "false",
            ],
        ),
    );
    let env = execute(&ctx, &request("runs:list", &[]));
    assert!(env.ok);
    let runs = env.data["runs"].as_array().unwrap();
    assert!(runs.iter().any(|run| run["run_id"] == "test-run-1"));
    let env = execute(&ctx, &request("runs:get", &["--run", "test-run-1"]));
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.data["operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|op| op["operation_type"] == "policy:set"));
    let env = execute(&ctx, &request("runs:get", &["--run", "no-such-run"]));
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "RUN_NOT_FOUND");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn runs_clear_clears_logs_without_side_effects() {
    let (base_empty, ctx_empty) = temp_ctx("runs-clear-empty");
    let env = execute(&ctx_empty, &request("runs:clear", &[]));
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["deleted"], 0);
    assert_eq!(env.data["domain"], "runs");
    assert!(!ctx_empty.data_dir.exists(), "无库时不应创建 data_dir");
    let _ = std::fs::remove_dir_all(&base_empty);

    let (base, ctx) = temp_ctx("runs-clear-populated");
    let conn = store::open_write(&ctx).unwrap();
    let mut custom_settings = store::default_settings();
    custom_settings.offline = true;
    custom_settings.log_retention_days = 45;
    store::write_settings(&conn, &custom_settings).unwrap();
    drop(conn);

    execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.codex.enabled",
                "--value",
                "false",
            ],
        ),
    );
    let list_before = execute(&ctx, &request("runs:list", &[]));
    assert!(list_before.ok);
    let runs_before = list_before.data["runs"].as_array().unwrap();
    assert!(!runs_before.is_empty(), "清空前应有任务记录");

    let clear_env = execute(&ctx, &request("runs:clear", &[]));
    assert!(clear_env.ok, "{:?}", clear_env.errors);
    let deleted = clear_env.data["deleted"].as_i64().unwrap();
    assert!(deleted > 0, "应报告已删除的记录条数");

    let list_after = execute(&ctx, &request("runs:list", &[]));
    assert!(list_after.ok);
    let runs_after = list_after.data["runs"].as_array().unwrap();
    assert_eq!(runs_after.len(), 0, "清空后任务记录列表应为空");

    let settings_after = store::effective_settings(&ctx).unwrap();
    assert_eq!(settings_after.offline, true, "app_settings 偏好应保留");
    assert_eq!(
        settings_after.log_retention_days, 45,
        "app_settings 保留天数应保留"
    );

    let policy_env = execute(&ctx, &request("policy:get", &[]));
    assert!(policy_env.ok);
    assert_eq!(
        policy_env.data["effective"]["agents"]["codex"]["enabled"],
        false
    );

    let clear_again = execute(&ctx, &request("runs:clear", &[]));
    assert!(clear_again.ok);
    assert_eq!(clear_again.data["deleted"], 0);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn unimplemented_domains_answer_clearly_and_unknown_rejected() {
    let (base, ctx) = temp_ctx("unimplemented");
    let env = execute(&ctx, &request("agents:status", &[]));
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["domain"], "agents");
    let claude = env.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == "agents:claude-code")
        .unwrap();
    assert_eq!(claude["observed"]["definition_origin"], "builtin");
    assert_eq!(claude["observed"]["command_sources"]["install"], "builtin");
    assert_eq!(claude["spec"]["preferred_provider"], "native");
    assert!(claude["spec"]["providers"]["native"].is_object());
    assert!(!claude.to_string().contains("brew"));
    let env = execute(&ctx, &request("bogus:status", &[]));
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "UNKNOWN_DOMAIN");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn agents_native_plan_dry_run_apply_and_audit() {
    let (base, ctx) = temp_ctx("agents-native");
    set_agent(&ctx, "local-agent", "agent-test-bin", "mkdir -p \"$HOME/.local/bin\"; printf '#!/bin/sh\\n' > \"$HOME/.local/bin/agent-test-bin\"; chmod +x \"$HOME/.local/bin/agent-test-bin\"");
    set_agent(&ctx, "broken-agent", "broken-test-bin", "exit 7");

    let plan = execute(
        &ctx,
        &request(
            "agents:plan",
            &["--only", "agents:local-agent", "--action", "install"],
        ),
    );
    assert!(plan.ok, "{:?}", plan.errors);
    assert_eq!(plan.data["actions"][0]["cmd"], "mkdir -p \"$HOME/.local/bin\"; printf '#!/bin/sh\\n' > \"$HOME/.local/bin/agent-test-bin\"; chmod +x \"$HOME/.local/bin/agent-test-bin\"");

    let before = execute(&ctx, &request("runs:list", &[]));
    let dry = execute(
        &ctx,
        &request(
            "agents:apply",
            &[
                "--only",
                "agents:local-agent",
                "--action",
                "install",
                "--dry-run",
            ],
        ),
    );
    assert!(dry.ok, "{:?}", dry.errors);
    assert_eq!(dry.data["dry_run"], true);
    let after = execute(&ctx, &request("runs:list", &[]));
    assert_eq!(before.data, after.data);
    assert!(!ctx.data_dir.join("agent-manager.apply.lock").exists());
    assert!(!ctx.home_dir.join(".local/bin/agent-test-bin").exists());

    let offline = execute(
        &ctx,
        &request(
            "agents:apply",
            &[
                "--only",
                "agents:local-agent",
                "--action",
                "install",
                "--offline",
            ],
        ),
    );
    assert!(!offline.ok);
    assert_eq!(offline.errors[0].code, "AGENT_ERROR");
    assert_eq!(before.data, execute(&ctx, &request("runs:list", &[])).data);
    assert!(!ctx.home_dir.join(".local/bin/agent-test-bin").exists());
    assert!(!ctx.data_dir.join("agent-manager.apply.lock").exists());

    let disabled_value = serde_json::to_string(&false).unwrap();
    let disabled = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.local-agent.enabled",
                "--value",
                &disabled_value,
            ],
        ),
    );
    assert!(disabled.ok, "{:?}", disabled.errors);
    let status = execute(&ctx, &request("agents:status", &[]));
    let local = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == "agents:local-agent")
        .unwrap();
    assert_eq!(local["policy"]["enabled"], false);
    assert_eq!(local["state"], "disabled");
    assert!(local["actions"].as_array().unwrap().is_empty());
    let blocked = execute(
        &ctx,
        &request(
            "agents:plan",
            &["--only", "agents:local-agent", "--action", "install"],
        ),
    );
    assert!(!blocked.ok);
    assert_eq!(blocked.errors[0].code, "AGENT_ERROR");

    let enabled_value = serde_json::to_string(&true).unwrap();
    let enabled = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.local-agent.enabled",
                "--value",
                &enabled_value,
            ],
        ),
    );
    assert!(enabled.ok, "{:?}", enabled.errors);

    let applied = execute(
        &ctx,
        &request(
            "agents:apply",
            &["--only", "agents:local-agent", "--action", "install"],
        ),
    );
    assert!(applied.ok, "{:?}", applied.errors);
    assert!(applied.lock.held);
    let runs = execute(&ctx, &request("runs:get", &["--run", "test-run-1"]));
    assert!(runs.data["operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|op| op["operation_type"] == "agents:install"));

    let failed = execute(
        &ctx,
        &Request {
            command: "agents:apply".into(),
            args: vec![
                "--only".into(),
                "agents:broken-agent".into(),
                "--action".into(),
                "install".into(),
            ],
            run_id: "failed-agent".into(),
        },
    );
    assert!(!failed.ok);
    assert_eq!(failed.errors[0].code, "AGENT_ERROR");
    assert!(!failed.errors[0].message.contains("exit 7"));
    let failed_run = execute(
        &ctx,
        &request_with_run("runs:get", &["--run", "failed-agent"], "failed-agent"),
    );
    assert!(failed_run.data["operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|op| op["operation_type"] == "agents:install"));

    let invalid = execute(
        &ctx,
        &request(
            "agents:plan",
            &["--only", "agents:local-agent", "--action", "version"],
        ),
    );
    assert!(!invalid.ok);
    assert_eq!(invalid.errors[0].code, "AGENT_ERROR");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn agent_apply_rejects_contended_lock_without_execution_or_log() {
    let (base, ctx) = temp_ctx("agents-lock");
    set_agent(
        &ctx,
        "locked-agent",
        "locked-test-bin",
        "printf executed > \"$HOME/contended-marker\"; exit 0",
    );
    let lock_path = ctx.data_dir.join("agent-manager.apply.lock");
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    lock_file.lock_exclusive().unwrap();

    let worker_ctx = Context {
        data_dir: ctx.data_dir.clone(),
        home_dir: ctx.home_dir.clone(),
    };
    let worker = std::thread::spawn(move || {
        execute(
            &worker_ctx,
            &request_with_run(
                "agents.apply",
                &["--only", "agents:locked-agent", "--action", "install"],
                "contended-agent",
            ),
        )
    });
    let env = worker.join().unwrap();
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "AGENT_ERROR");
    assert!(!env.lock.held);
    assert!(!ctx.home_dir.join("contended-marker").exists());
    let runs = execute(
        &ctx,
        &request_with_run("runs:get", &["--run", "contended-agent"], "contended-agent"),
    );
    assert!(!runs.ok);
    assert_eq!(runs.errors[0].code, "RUN_NOT_FOUND");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn agent_dry_run_without_database_does_not_create_lock_file() {
    let (base, ctx) = temp_ctx("agents-dry-run-no-db");
    let env = execute(
        &ctx,
        &request(
            "agents.apply",
            &[
                "--only",
                "agents:not-in-database",
                "--action",
                "install",
                "--dry-run",
            ],
        ),
    );
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "AGENT_ERROR");
    assert!(!ctx.data_dir.exists());
    assert!(!ctx.data_dir.join("agent-manager.apply.lock").exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn agent_status_classifies_unsupported_install_and_skips_tombstones() {
    let (base, ctx) = temp_ctx("agents-source");
    let brew_bin = base
        .join("Cellar")
        .join("demo")
        .join("bin")
        .join("demo-agent");
    std::fs::create_dir_all(brew_bin.parent().unwrap()).unwrap();
    std::fs::write(&brew_bin, "#!/bin/sh\n").unwrap();
    set_agent_definition(
        &ctx,
        "brew-agent",
        json!({
            "display_name": "Brew Agent",
            "bin": brew_bin,
            "preferred_provider": "native",
            "providers": {"native": {
                "managed": true,
                "install": "printf install > \"$HOME/brew-marker\"",
                "upgrade": "printf upgrade > \"$HOME/brew-marker\"",
                "uninstall": "printf uninstall > \"$HOME/brew-marker\""
            }}
        }),
    );
    set_agent_definition(&ctx, "removed-agent", json!({"_delete": true}));
    let status = execute(&ctx, &request("agents:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    let brew = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == "agents:brew-agent")
        .unwrap();
    assert_eq!(brew["state"], "unmanaged-detected");
    assert!(brew["actions"].as_array().unwrap().is_empty());
    assert_eq!(brew["observed"]["providers"][0], "brew");
    assert_eq!(brew["observed"]["classified"]["unsupported"][0], "brew");
    assert_eq!(brew["observed"]["preferred_active"], false);
    for command in ["agents:plan", "agents.apply"] {
        let request_args = ["--only", "agents:brew-agent", "--action", "upgrade"];
        let result = execute(
            &ctx,
            &request_with_run(command, &request_args, "brew-direct-apply"),
        );
        assert!(!result.ok);
        assert_eq!(result.errors[0].code, "AGENT_ERROR");
        assert!(!result.lock.held);
    }
    let apply = execute(
        &ctx,
        &request_with_run(
            "agents:apply",
            &["--only", "agents:brew-agent", "--action", "upgrade"],
            "brew-apply",
        ),
    );
    assert!(!apply.ok);
    assert!(!ctx.home_dir.join("brew-marker").exists());
    let brew_run = execute(
        &ctx,
        &request_with_run("runs:get", &["--run", "brew-apply"], "brew-apply"),
    );
    assert!(!brew_run.ok);
    assert_eq!(brew_run.errors[0].code, "RUN_NOT_FOUND");
    assert!(status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .all(|element| element["id"] != "agents:removed-agent"));
    let _ = std::fs::remove_dir_all(&base);
}

fn request_with_run(command: &str, args: &[&str], run_id: &str) -> Request {
    Request {
        command: command.into(),
        args: args.iter().map(|s| (*s).into()).collect(),
        run_id: run_id.into(),
    }
}

#[test]
fn secret_rules_and_no_write_guarantee() {
    let (base, ctx) = temp_ctx("secrets");
    let samples = vec![
        format!("sk-{}", "x".repeat(20)),
        format!("ghp_{}", "x".repeat(20)),
        format!("AIza{}", "x".repeat(20)),
        format!("AKIA{}", "A1B2C3D4E5F6G7H8"),
        format!("-----BEGIN {} PRIVATE KEY-----", "RSA"),
        format!(r#"{{"accessToken":"ace_{}"}}"#, "x".repeat(12)),
    ];
    for sample in &samples {
        let env = execute(
            &ctx,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "agents.secret",
                    "--value",
                    &serde_json::to_string(&sample).unwrap(),
                ],
            ),
        );
        assert_eq!(env.errors[0].code, "SECRET_INPUT_REJECTED");
    }
    assert!(!ctx.data_dir.exists());
    for value in ["${ENV_NAME}", "YOUR_TOKEN", "secrets://openai/key"] {
        let env = execute(
            &ctx,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "agents.reference",
                    "--value",
                    &serde_json::to_string(value).unwrap(),
                ],
            ),
        );
        assert!(env.ok, "{:?}", env.errors);
    }
    let mixed = format!("YOUR_TOKEN-{}", "ghp_".to_owned() + &"x".repeat(20));
    let env = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.mixed",
                "--value",
                &serde_json::to_string(&mixed).unwrap(),
            ],
        ),
    );
    assert_eq!(env.errors[0].code, "SECRET_INPUT_REJECTED");
    let _ = std::fs::create_dir_all(&ctx.data_dir);
    let ok = execute(
        &ctx,
        &request(
            "policy:set",
            &["--layer", "local", "--key", "agents.ok", "--value", "true"],
        ),
    );
    assert!(ok.ok);
    let before = store::effective_policy(&ctx).unwrap();
    let count_before: i64 = store::open_readonly(&ctx)
        .unwrap()
        .unwrap()
        .query_row("SELECT count(*) FROM operation_logs", [], |r| r.get(0))
        .unwrap();
    let env = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.secret",
                "--value",
                &serde_json::to_string(&samples[0]).unwrap(),
            ],
        ),
    );
    assert_eq!(env.errors[0].code, "SECRET_INPUT_REJECTED");
    let after = store::effective_policy(&ctx).unwrap();
    let count_after: i64 = store::open_readonly(&ctx)
        .unwrap()
        .unwrap()
        .query_row("SELECT count(*) FROM operation_logs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(count_before, count_after);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn dry_run_and_invalid_input_do_not_initialize_store() {
    let (root, context) = temp_ctx("guard");
    let dry = execute(
        &context,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.codex.enabled",
                "--value",
                "false",
                "--dry-run",
            ],
        ),
    );
    assert!(dry.ok);
    let invalid = execute(
        &context,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.bad key",
                "--value",
                "true",
            ],
        ),
    );
    assert!(!invalid.ok);
    assert!(!context.data_dir.exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn first_write_round_trips_and_preserves_builtin_domains() {
    let (root, context) = temp_ctx("roundtrip");
    let result = execute(
        &context,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.codex.display_name",
                "--value",
                "\"Custom Codex\"",
            ],
        ),
    );
    assert!(result.ok, "{:?}", result.errors);
    let policy = store::effective_policy(&context).unwrap();
    assert_eq!(
        policy["agents"]["definitions"]["codex"]["display_name"],
        "Custom Codex"
    );
    assert!(policy["mcp"]["definitions"].is_object());
    assert!(policy["skills"]["repositories"].is_object());
    let _ = std::fs::remove_dir_all(root);
}
