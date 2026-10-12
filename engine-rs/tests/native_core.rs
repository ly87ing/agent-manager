use agent_manager_engine::{execute, store, Context, Request};
use serde_json::json;
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
    let mut policy = store::effective_policy(ctx).unwrap();
    policy["agents"]["definitions"][id] = value;
    store::write_policy(&store::open_write(ctx).unwrap(), &policy).unwrap();
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
fn agents_status_reads_current_client_config_redacted_and_zero_write() {
    let (base, ctx) = temp_ctx("agents-current-config");
    std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
    let bytes = br#"{"env":{"ANTHROPIC_BASE_URL":"https://relay.example.com","ANTHROPIC_AUTH_TOKEN":"status-secret","ANTHROPIC_MODEL":"claude-observed"}}"#;
    let path = ctx.home_dir.join(".claude/settings.json");
    std::fs::write(&path, bytes).unwrap();

    let env = execute(&ctx, &request("resources:targets", &[]));
    assert!(env.ok, "{:?}", env.errors);
    assert!(!env.data.to_string().contains("status-secret"));
    assert!(env.data["targets"]
        .as_array()
        .unwrap()
        .iter()
        .all(|target| target.get("current_config").is_none()));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(
        !ctx.data_dir.exists(),
        "只读 status 不得创建数据库或数据目录"
    );
    let _ = std::fs::remove_dir_all(base);
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
    assert!(
        effective["agents"]["definitions"].get("opencode").is_none(),
        "新安装只内置 Claude Code 与 Codex"
    );
    // 技能仓库只作推荐来源（skills:status 的 recommendations），内置 policy 不再带集合，
    // 首次写库也就不会把它固化成用户集合
    assert_eq!(effective["skills"]["repositories"], serde_json::json!({}));
    assert!(effective["skills"].get("agent-skills").is_none());
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
                "skills.enabled",
                "--value",
                "false",
            ],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
    assert!(store::db_path(&ctx).exists());
    let effective = store::effective_policy(&ctx).unwrap();
    assert_eq!(effective["skills"]["enabled"], false);
    // 内置默认已固化进库
    assert!(effective["agents"]["definitions"]["claude-code"].is_object());
    // 只读重开（等价重启进程）仍是新值
    let env = execute(&ctx, &request("policy:get", &[]));
    assert_eq!(env.data["effective"]["skills"]["enabled"], false);
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
fn dry_run_set_is_rejected_and_initializes_no_store() {
    let (base, ctx) = temp_ctx("dryrun");
    let env = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.enabled",
                "--value",
                "false",
                "--dry-run",
            ],
        ),
    );
    assert!(!env.ok, "--dry-run 已退役，必须拒绝: {:?}", env.data);
    assert_eq!(env.errors[0].code, "USAGE_ERROR");
    // 零目标写：不得建库
    assert!(!store::db_path(&ctx).exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn replace_semantics_and_delete_roundtrip() {
    let (base, ctx) = temp_ctx("replace");
    // 通用 policy:set/_replace/delete 语义。原先拿 mcp.definitions 作例子；MCP 定义现在
    // 写入即转成「未分配的已保存定义」、不再留在策略里，改用中性的 demo 键验证同一语义。
    execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "demo",
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
                "demo.definitions",
                "--value",
                "{\"_replace\":true,\"srv\":{\"command\":\"b\"}}",
            ],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
    let effective = store::effective_policy(&ctx).unwrap();
    assert_eq!(effective["demo"]["definitions"]["srv"]["command"], "b");
    assert!(effective["demo"]["definitions"]["srv"]
        .get("keep")
        .is_none());
    // delete 存在的键成功、再删报错
    let env = execute(
        &ctx,
        &request(
            "policy:delete",
            &["--layer", "local", "--key", "demo.definitions.srv"],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
    let env = execute(
        &ctx,
        &request(
            "policy:delete",
            &["--layer", "local", "--key", "demo.definitions.srv"],
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
        vec!["--key", "skills.missing"],
        vec!["--layer", "synced", "--key", "skills.missing"],
    ] {
        let env = execute(&ctx, &request("policy:delete", &args));
        assert!(!env.ok);
        assert_eq!(env.errors[0].code, "USAGE_ERROR");
    }
    assert!(!store::db_path(&ctx).exists(), "非法写参数不应初始化数据库");

    let set = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.keep",
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
        vec!["--key", "skills.keep"],
        vec!["--layer", "synced", "--key", "skills.keep"],
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
                "skills.enabled",
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
    assert!(!store::db_path(&ctx_empty).exists(), "无库时不应创建数据库");
    let _ = std::fs::remove_dir_all(&base_empty);

    let (base, ctx) = temp_ctx("runs-clear-populated");
    let conn = store::open_write(&ctx).unwrap();
    let mut custom_settings = store::default_settings();
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
                "skills.enabled",
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
    assert_eq!(
        settings_after.log_retention_days, 45,
        "app_settings 保留天数应保留"
    );

    let policy_env = execute(&ctx, &request("policy:get", &[]));
    assert!(policy_env.ok);
    assert_eq!(policy_env.data["effective"]["skills"]["enabled"], false);

    let clear_again = execute(&ctx, &request("runs:clear", &[]));
    assert!(clear_again.ok);
    assert_eq!(clear_again.data["deleted"], 0);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn unimplemented_domains_answer_clearly_and_unknown_rejected() {
    let (base, ctx) = temp_ctx("unimplemented");
    let env = execute(&ctx, &request("agents:status", &[]));
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "FEATURE_RETIRED");
    let env = execute(&ctx, &request("bogus:status", &[]));
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "UNKNOWN_DOMAIN");
    let _ = std::fs::remove_dir_all(&base);
}

// 2026-09-12 裁决退役面：tools/snapshots 域整体 UNKNOWN_DOMAIN；
// system:converge 命中已知域的未知动词错误；system:refresh-upstream 已接线
// （探测语义见 tests/native_upstream.rs），此处仅验证 offline 拒绝不回退到
// DOMAIN_NOT_IMPLEMENTED。
#[test]
fn retired_domains_and_verbs_are_rejected_clearly() {
    let (base, ctx) = temp_ctx("retired");
    for command in [
        "tools:status",
        "tools:plan",
        "tools:apply",
        "snapshots:list",
        "snapshots:restore",
    ] {
        let env = execute(&ctx, &request(command, &[]));
        assert!(!env.ok, "{command} 必须失败");
        assert_eq!(
            env.errors[0].code, "UNKNOWN_DOMAIN",
            "{command} 应为 UNKNOWN_DOMAIN: {:?}",
            env.errors
        );
    }
    let env = execute(&ctx, &request("system:converge", &[]));
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "UNKNOWN_VERB", "{:?}", env.errors);
    let env = execute(&ctx, &request("system:refresh-upstream", &["--offline"]));
    assert!(!env.ok);
    assert_eq!(
        env.errors[0].code, "OFFLINE_NETWORK_DENIED",
        "refresh-upstream 已接线：offline 拒绝而非未接线: {:?}",
        env.errors
    );
    assert!(!store::db_path(&ctx).exists(), "offline 拒绝不得建库");
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
                    "skills.secret",
                    "--value",
                    &serde_json::to_string(&sample).unwrap(),
                ],
            ),
        );
        assert_eq!(env.errors[0].code, "SECRET_INPUT_REJECTED");
    }
    assert!(
        !store::db_path(&ctx).exists(),
        "拒绝明文写动作不应初始化数据库"
    );
    for value in ["${ENV_NAME}", "YOUR_TOKEN", "secrets://openai/key"] {
        let env = execute(
            &ctx,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "skills.reference",
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
                "skills.mixed",
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
            &["--layer", "local", "--key", "skills.ok", "--value", "true"],
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
                "skills.secret",
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
                "skills.enabled",
                "--value",
                "false",
                "--dry-run",
            ],
        ),
    );
    assert!(!dry.ok, "--dry-run 已退役，必须拒绝: {:?}", dry.data);
    assert_eq!(dry.errors[0].code, "USAGE_ERROR");
    let invalid = execute(
        &context,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.bad key",
                "--value",
                "true",
            ],
        ),
    );
    assert!(!invalid.ok);
    assert!(
        !store::db_path(&context).exists(),
        "非法写参数不应初始化数据库"
    );
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
                "skills.label",
                "--value",
                "\"Custom Codex\"",
            ],
        ),
    );
    assert!(result.ok, "{:?}", result.errors);
    let policy = store::effective_policy(&context).unwrap();
    assert_eq!(policy["skills"]["label"], "Custom Codex");
    assert!(policy["mcp"]["definitions"].is_object());
    assert!(policy["skills"]["repositories"].is_object());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn metadata_status_skips_versions_and_preserves_the_store() {
    let (root, ctx) = temp_ctx("metadata-status");
    set_agent(&ctx, "fixture", "fixture-missing", "true");
    let before = std::fs::read(store::db_path(&ctx)).unwrap();
    let status = execute(&ctx, &request("resources:targets", &[]));
    assert!(status.ok, "{:?}", status.errors);
    assert!(status.data["targets"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["observed"]["version"].is_null()));
    assert_eq!(std::fs::read(store::db_path(&ctx)).unwrap(), before);
    assert!(!ctx.home_dir.exists());
    assert!(!execute(&ctx, &request("resources:targets", &["--unknown"])).ok);
    std::fs::remove_dir_all(root).unwrap();
}
