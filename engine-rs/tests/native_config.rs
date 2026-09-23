//! 配置备份：导出只读、导入整体替换、坏内容一律拒绝且不留半份。

use agent_manager_engine::{execute, store, Context, Request};
use std::path::PathBuf;

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    let base = std::env::temp_dir().join(format!("am-config-test-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&base);
    let ctx = Context {
        data_dir: base.join("data"),
        home_dir: base.join("home"),
    };
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    (base, ctx)
}

fn request(command: &str, args: &[&str]) -> Request {
    Request {
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        run_id: "config-test".to_string(),
    }
}

#[test]
fn export_is_read_only_and_roundtrips_definitions_and_sources() {
    let (root, ctx) = temp_ctx("roundtrip");

    // 缺库导出：零写，拿到的是内置默认
    let fresh = execute(&ctx, &request("config:export", &[]));
    assert!(fresh.ok, "{:?}", fresh.errors);
    assert_eq!(fresh.data["format"], "agent-manager/config-backup/v1");
    assert!(!ctx.data_dir.exists(), "导出不得创建数据目录");

    // 造一份本机配置：一个 MCP 定义 + 一个规则模板
    assert!(
        execute(
            &ctx,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "mcp.definitions.demo",
                    "--value",
                    r#"{"command":"npx","args":["-y","demo"],"env":{"DEMO_TOKEN":"${DEMO_TOKEN}"},"_delete":false,"_replace":true}"#,
                ],
            ),
        )
        .ok
    );
    assert!(
        execute(
            &ctx,
            &request(
                "rules:save-template",
                &["--name", "我的规则模板", "--value", "我的共享规则\n"],
            ),
        )
        .ok
    );

    let exported = execute(&ctx, &request("config:export", &[]));
    assert!(exported.ok, "{:?}", exported.errors);
    let payload = serde_json::to_string(&exported.data).unwrap();
    assert!(payload.contains("${DEMO_TOKEN}"), "定义以引用形式进备份");
    assert!(payload.contains("我的共享规则"));
    // 规则不再是来源：备份只带模板，不带 rules.* 来源键
    assert!(exported.data["sources"].get("rules.shared").is_none());
    assert_eq!(exported.data["rule_templates"][0]["name"], "我的规则模板");
    // 分配记录与本机落地状态不进备份
    assert!(exported.data.get("bindings").is_none());

    // 换一台「新机器」：空环境导入这份备份
    let (root2, ctx2) = temp_ctx("restore");
    let restore = execute(&ctx2, &request("config:import", &["--value", &payload]));
    assert!(restore.ok, "{:?}", restore.errors);

    let after = execute(&ctx2, &request("config:export", &[]));
    assert_eq!(
        after.data["policy"]["mcp"]["definitions"]["demo"]["command"],
        "npx"
    );
    assert_eq!(after.data["rule_templates"][0]["name"], "我的规则模板");
    assert_eq!(after.data["rule_templates"][0]["body"], "我的共享规则\n");

    // 恢复的是定义，不是落地状态：还没应用过，目标文件不存在
    assert!(!ctx2.home_dir.join(".claude/CLAUDE.md").exists());

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(root2);
}

#[test]
fn import_rejects_bad_payloads_without_partial_writes() {
    let (root, ctx) = temp_ctx("reject");
    // 先放一份本机配置，确认失败后它不被改动
    assert!(
        execute(
            &ctx,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "mcp.definitions.keep",
                    "--value",
                    r#"{"command":"npx","args":[],"_delete":false,"_replace":true}"#
                ],
            ),
        )
        .ok
    );
    let before = execute(&ctx, &request("config:export", &[]));
    let before_policy = serde_json::to_string(&before.data["policy"]).unwrap();

    let cases: Vec<(&str, String)> = vec![
        ("不是 JSON", "not json".to_string()),
        (
            "格式标识不认识",
            r#"{"format":"other/v1","policy":{}}"#.to_string(),
        ),
        (
            "缺 policy",
            r#"{"format":"agent-manager/config-backup/v1"}"#.to_string(),
        ),
        (
            "明文凭据",
            r#"{"format":"agent-manager/config-backup/v1","policy":{"mcp":{"definitions":{"x":{"command":"npx","args":[],"env":{"K":"sk-ant-0123456789abcdefghijklmnop"}}}}}}"#.to_string(),
        ),
        (
            "来源键不认识",
            r#"{"format":"agent-manager/config-backup/v1","policy":{},"sources":{"evil.key":"x"}}"#.to_string(),
        ),
        (
            "hooks 来源结构非法",
            r#"{"format":"agent-manager/config-backup/v1","policy":{},"sources":{"hooks.claude":"{\"hooks\":{\"Stop\":\"not-an-array\"}}"}}"#.to_string(),
        ),
    ];
    for (label, payload) in cases {
        let env = execute(&ctx, &request("config:import", &["--value", &payload]));
        assert!(!env.ok, "[{label}] 必须被拒绝");
        assert_eq!(env.errors[0].code, "CONFIG_ERROR", "[{label}]");
    }

    let after = execute(&ctx, &request("config:export", &[]));
    assert_eq!(
        serde_json::to_string(&after.data["policy"]).unwrap(),
        before_policy,
        "任何一次失败都不得改动本机配置"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn export_takes_no_args_and_import_needs_value() {
    let (root, ctx) = temp_ctx("args");
    let bad_export = execute(&ctx, &request("config:export", &["--only", "x"]));
    assert!(!bad_export.ok);
    let bad_import = execute(&ctx, &request("config:import", &[]));
    assert!(!bad_import.ok);
    assert!(!store::db_path(&ctx).exists(), "参数反例不得建库");
    let _ = std::fs::remove_dir_all(root);
}

/// 备份永不带凭据取值；外部密钥文件独立迁移，旧版含密钥备份拒绝导入。
#[test]
fn backup_excludes_secret_values_and_rejects_legacy_secret_payload() {
    const SECRET: &str = "secret-value-under-test";
    let (root_a, ctx_a) = temp_ctx("secrets-src");
    let (root_b, ctx_b) = temp_ctx("secrets-dst");

    let secrets_file = root_a.join("private-secrets.env");
    std::fs::write(&secrets_file, "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&secrets_file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(
        execute(
            &ctx_a,
            &request(
                "secrets:source",
                &["--path", secrets_file.to_str().unwrap()],
            ),
        )
        .ok
    );

    assert!(
        execute(
            &ctx_a,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "mcp.definitions.demo",
                    "--value",
                    r#"{"command":"npx","args":["-y","demo"],"env":{"K":"${MY_TOKEN}"},"_delete":false,"_replace":true}"#,
                ],
            ),
        )
        .ok
    );
    assert!(
        execute(
            &ctx_a,
            &request("secrets:set", &["--var", "MY_TOKEN", "--value", SECRET]),
        )
        .ok
    );

    let exported = execute(&ctx_a, &request("config:export", &[]));
    assert!(exported.ok, "{:?}", exported.errors);
    assert!(exported.data.get("secrets").is_none());
    assert!(!serde_json::to_string(&exported.data)
        .unwrap()
        .contains(SECRET));

    // 另一台机器可以恢复定义，但凭据仍由它自己的外部文件提供。
    let payload = serde_json::to_string(&exported.data).unwrap();
    let imported = execute(&ctx_b, &request("config:import", &["--value", &payload]));
    assert!(imported.ok, "{:?}", imported.errors);

    let status = execute(&ctx_b, &request("secrets:status", &[]));
    let observed = &status.data["elements"][0]["observed"];
    assert_eq!(observed["unresolved"], 1);
    assert_eq!(observed["variables"][0]["stored"], false);

    let legacy = serde_json::json!({
        "format": "agent-manager/config-backup/v1",
        "policy": {},
        "sources": {},
        "secrets": {"MY_TOKEN": SECRET},
    })
    .to_string();
    let rejected = execute(&ctx_b, &request("config:import", &["--value", &legacy]));
    assert!(!rejected.ok);
    assert!(rejected.errors[0].message.contains("外部密钥文件"));
    assert!(!serde_json::to_string(&rejected).unwrap().contains(SECRET));

    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

/// 老备份没有 secrets 段：按空处理照常导入，不报错。
#[test]
fn backup_without_secrets_section_still_imports() {
    let (root, ctx) = temp_ctx("legacy-backup");
    let payload = serde_json::json!({
        "format": "agent-manager/config-backup/v1",
        "policy": {},
        "sources": {},
    })
    .to_string();
    let env = execute(&ctx, &request("config:import", &["--value", &payload]));
    assert!(env.ok, "{:?}", env.errors);
    let _ = std::fs::remove_dir_all(&root);
}

/// secrets 段只能省略或为空；任何明文或坏结构都整份拒绝。
#[test]
fn backup_with_malformed_secrets_is_rejected_whole() {
    let (root, ctx) = temp_ctx("bad-secrets");
    for bad in [
        serde_json::json!({"format":"agent-manager/config-backup/v1","policy":{},"sources":{},"secrets":{"bad-name":"v"}}),
        serde_json::json!({"format":"agent-manager/config-backup/v1","policy":{},"sources":{},"secrets":{"OK":1}}),
        serde_json::json!({"format":"agent-manager/config-backup/v1","policy":{},"sources":{},"secrets":"nope"}),
    ] {
        let env = execute(
            &ctx,
            &request("config:import", &["--value", &bad.to_string()]),
        );
        assert!(!env.ok, "结构非法的 secrets 必须被拒绝");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 旧备份里的规则来源（rules.*）仍能导入：按旧规则渲染成「旧规则来源」模板，
/// 与内置相同的不生成；不写任何规则文件。
#[test]
fn legacy_rule_sources_in_backup_become_templates() {
    let (root, ctx) = temp_ctx("legacy-rules");
    let core = include_str!("../../agent-rule/rules/src/shared/core.md");
    let payload = serde_json::json!({
        "format": "agent-manager/config-backup/v1",
        "policy": {},
        "sources": {
            "rules.shared": core,
            "rules.claude": "只给 Claude\n",
            "rules.codex": "",
        },
    })
    .to_string();
    let env = execute(&ctx, &request("config:import", &["--value", &payload]));
    assert!(env.ok, "{:?}", env.errors);
    let status = execute(&ctx, &request("rules:status", &[]));
    let templates = status.data["templates"].as_array().unwrap();
    assert_eq!(
        templates.len(),
        1,
        "只有改过的 Claude 生成模板: {templates:?}"
    );
    assert_eq!(templates[0]["name"], "旧规则来源（Claude）");
    assert_eq!(templates[0]["body"], format!("{core}只给 Claude\n"));
    assert!(!ctx.home_dir.join(".claude/CLAUDE.md").exists());

    // 模板段里有疑似凭据：整份拒绝
    let bad = serde_json::json!({
        "format": "agent-manager/config-backup/v1",
        "policy": {},
        "rule_templates": [{"name": "x", "body": format!("sk-{}", "a".repeat(24))}],
    })
    .to_string();
    let env = execute(&ctx, &request("config:import", &["--value", &bad]));
    assert!(!env.ok);
    let status = execute(&ctx, &request("rules:status", &[]));
    assert_eq!(
        status.data["templates"].as_array().unwrap().len(),
        1,
        "失败不改库"
    );
    let _ = std::fs::remove_dir_all(root);
}
