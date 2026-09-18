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

    // 造一份本机配置：一个 MCP 定义 + 改过的规则来源
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
    let sources = execute(&ctx, &request("sources:get", &["--key", "rules"]));
    let revision = sources.data["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["key"] == "rules.shared")
        .unwrap()["revision"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        execute(
            &ctx,
            &request(
                "sources:set",
                &[
                    "--key",
                    "rules.shared",
                    "--value",
                    &format!(r#"{{"revision":"{revision}","value":"我的共享规则\n"}}"#),
                ],
            ),
        )
        .ok
    );

    let exported = execute(&ctx, &request("config:export", &[]));
    assert!(exported.ok, "{:?}", exported.errors);
    let payload = serde_json::to_string(&exported.data).unwrap();
    assert!(payload.contains("${DEMO_TOKEN}"), "定义以引用形式进备份");
    assert!(payload.contains("我的共享规则"));
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
    assert_eq!(after.data["sources"]["rules.shared"], "我的共享规则\n");

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

/// 备份带上凭据取值：换机器导入即可用。代价是这份文件本身就是凭据载体，
/// 因此同时断言取值不进操作记录。
#[test]
fn backup_carries_secret_values_and_restores_them_on_another_machine() {
    const SECRET: &str = "secret-value-under-test";
    let (root_a, ctx_a) = temp_ctx("secrets-src");
    let (root_b, ctx_b) = temp_ctx("secrets-dst");

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
    assert_eq!(exported.data["secrets"]["MY_TOKEN"], SECRET);

    // 另一台机器（全新上下文）导入后，变量当场可解析
    let payload = serde_json::to_string(&exported.data).unwrap();
    let imported = execute(&ctx_b, &request("config:import", &["--value", &payload]));
    assert!(imported.ok, "{:?}", imported.errors);

    let status = execute(&ctx_b, &request("secrets:status", &[]));
    let observed = &status.data["elements"][0]["observed"];
    assert_eq!(observed["unresolved"], 0, "导入后变量应当已可解析");
    assert_eq!(observed["variables"][0]["stored"], true);
    // status 永不回显取值
    assert!(!serde_json::to_string(&status.data)
        .unwrap()
        .contains(SECRET));

    // 审计只记变量名，不记取值
    let runs = execute(&ctx_b, &request("runs:list", &[]));
    assert!(!serde_json::to_string(&runs.data).unwrap().contains(SECRET));

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

/// secrets 段结构非法即整份拒绝，且不部分写入。
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
