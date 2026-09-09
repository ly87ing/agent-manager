use agent_manager_engine::{execute, store, Context, Request};
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
fn unimplemented_domains_answer_clearly_and_unknown_rejected() {
    let (base, ctx) = temp_ctx("unimplemented");
    let env = execute(&ctx, &request("agents:status", &[]));
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "DOMAIN_NOT_IMPLEMENTED");
    let env = execute(&ctx, &request("bogus:status", &[]));
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "UNKNOWN_DOMAIN");
    let _ = std::fs::remove_dir_all(&base);
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
