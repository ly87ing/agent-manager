use agent_manager_engine::{execute, store, Context, Request};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    let base =
        std::env::temp_dir().join(format!("am-native-sources-{}-{}", std::process::id(), tag));
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

fn request_owned(command: &str, args: Vec<String>) -> Request {
    Request {
        command: command.to_string(),
        args,
        run_id: "test-run-1".to_string(),
    }
}

fn sha256_hex(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn alias_value(command: &str) -> Value {
    json!({"aliases": {"hi": command}})
}

/// aliases.shell 当前 revision（缺库时为内置默认的 revision）。
fn alias_revision(ctx: &Context) -> String {
    let env = execute(ctx, &request("sources:get", &["--key", "aliases.shell"]));
    assert!(env.ok, "{:?}", env.errors);
    env.data["sources"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string()
}

/// 规则来源已退役：生效规则以各助手的规则文件为准（rules 域直接编辑、模板存
/// rule_templates）。rules 前缀与三个精确键在 get/set 上一律拒绝，且零写。
#[test]
fn rules_source_keys_are_retired_with_zero_writes() {
    let (base, ctx) = temp_ctx("rules-retired");
    let wrapper = json!({"revision": "0".repeat(64), "value": "团队规则\n"}).to_string();
    for key in ["rules", "rules.shared", "rules.claude", "rules.codex"] {
        let env = execute(&ctx, &request("sources:get", &["--key", key]));
        assert!(!env.ok, "{key} 已退役");
        assert_eq!(env.errors[0].code, "SOURCE_ERROR");
        assert!(env.errors[0].message.contains("规则来源已退役"), "{key}");
        let env = execute(
            &ctx,
            &request("sources:set", &["--key", key, "--value", &wrapper]),
        );
        assert!(!env.ok, "{key} 已退役");
        assert_eq!(env.errors[0].code, "SOURCE_ERROR");
        assert!(env.errors[0].message.contains("规则来源已退役"), "{key}");
        assert!(!env.errors[0].message.contains("团队规则"));
    }
    assert!(!ctx.data_dir.exists(), "拒绝不得创建数据目录");

    // 已有库：同样拒绝，库字节不变，不会建出 rule_sources 表
    let conn = store::open_write(&ctx).unwrap();
    drop(conn);
    let db = store::db_path(&ctx);
    let before = std::fs::read(&db).unwrap();
    let env = execute(
        &ctx,
        &request(
            "sources:set",
            &["--key", "rules.shared", "--value", &wrapper],
        ),
    );
    assert!(!env.ok);
    assert_eq!(std::fs::read(&db).unwrap(), before);
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let has_table: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='rule_sources'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(has_table, 0);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn get_rejects_nonconforming_args_with_source_error_and_zero_writes() {
    let (base, ctx) = temp_ctx("get-strict");
    let cases: Vec<Vec<&str>> = vec![
        vec![],
        vec!["--key"],
        vec!["--key", "--value"],
        vec!["--key", "aliases.nope"],
        vec!["--key", "ALIASES"],
        vec!["--key", "aliases", "--key", "aliases"],
        vec!["--key", "aliases", "positional"],
        vec!["--key", "aliases", "--offline"],
        vec!["--key", "aliases", "--value", "{}"],
        vec!["--key", "aliases", "--dry-run"],
    ];
    for args in cases {
        let env = execute(&ctx, &request("sources:get", &args));
        assert!(!env.ok, "args {args:?} 应失败");
        assert_eq!(env.errors[0].code, "SOURCE_ERROR", "args {args:?}");
    }
    assert!(!ctx.data_dir.exists());
    assert!(!store::db_path(&ctx).exists());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn set_dry_run_is_rejected_before_any_write() {
    let (base, ctx) = temp_ctx("dry-run");
    let current = execute(&ctx, &request("sources:get", &["--key", "aliases.shell"]));
    let revision = current.data["sources"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    let default_value = current.data["sources"][0]["value"].clone();
    let wrapper = json!({ "revision": revision, "value": alias_value("echo dry") }).to_string();
    let env = execute(
        &ctx,
        &request(
            "sources:set",
            &["--key", "aliases.shell", "--value", &wrapper, "--dry-run"],
        ),
    );
    assert!(!env.ok, "--dry-run 已退役，必须拒绝: {:?}", env.data);
    assert_eq!(env.errors[0].code, "SOURCE_ERROR");
    // 零目标写：无 DB/表/apply lock/助手目标文件
    assert!(!store::db_path(&ctx).exists());
    assert!(!ctx.home_dir.exists());
    assert!(!ctx.data_dir.join("agent-manager.apply.lock").exists());
    let after = execute(&ctx, &request("sources:get", &["--key", "aliases.shell"]));
    assert_eq!(after.data["sources"][0]["value"], default_value);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn set_rejects_invalid_secret_nul_and_stale_inputs_without_writes() {
    let (base, ctx) = temp_ctx("set-invalid");
    let good_revision = alias_revision(&ctx);
    let secret_body = format!("sk-{}", "a".repeat(24));

    let cases: Vec<Vec<String>> = vec![
        // 前缀/未知 key
        vec!["--key".into(), "aliases".into()],
        vec![
            "--key".into(),
            "aliases.nope".into(),
            "--value".into(),
            json!({"revision": good_revision, "value": {"aliases": {}}}).to_string(),
        ],
        // 缺 --value / 缺值 / 位置参数 / 未知 flag / 重复 flag
        vec!["--key".into(), "aliases.shell".into()],
        vec!["--key".into(), "aliases.shell".into(), "--value".into()],
        vec!["positional".into()],
        vec![
            "--key".into(),
            "aliases.shell".into(),
            "--value".into(),
            "{}".into(),
            "--offline".into(),
        ],
        vec![
            "--key".into(),
            "aliases.shell".into(),
            "--key".into(),
            "aliases".into(),
            "--value".into(),
            "{}".into(),
        ],
        // 非法 wrapper
        vec![
            "--key".into(),
            "aliases.shell".into(),
            "--value".into(),
            "{not json".into(),
        ],
        vec![
            "--key".into(),
            "aliases.shell".into(),
            "--value".into(),
            "true".into(),
        ],
        vec![
            "--key".into(),
            "aliases.shell".into(),
            "--value".into(),
            "[]".into(),
        ],
        vec![
            "--key".into(),
            "aliases.shell".into(),
            "--value".into(),
            json!({"revision": good_revision, "value": {"aliases": {}}, "extra": 1}).to_string(),
        ],
        vec![
            "--key".into(),
            "aliases.shell".into(),
            "--value".into(),
            json!({"value": {"aliases": {}}}).to_string(),
        ],
        vec![
            "--key".into(),
            "aliases.shell".into(),
            "--value".into(),
            json!({"revision": good_revision}).to_string(),
        ],
        vec![
            "--key".into(),
            "aliases.shell".into(),
            "--value".into(),
            json!({"revision": 1, "value": {"aliases": {}}}).to_string(),
        ],
        vec![
            "--key".into(),
            "aliases.shell".into(),
            "--value".into(),
            json!({"revision": good_revision, "value": 1}).to_string(),
        ],
        // NUL 正文
        vec![
            "--key".into(),
            "aliases.shell".into(),
            "--value".into(),
            json!({"revision": good_revision, "value": alias_value("a\0b")}).to_string(),
        ],
        // 过期 revision
        vec![
            "--key".into(),
            "aliases.shell".into(),
            "--value".into(),
            json!({"revision": "0".repeat(64), "value": {"aliases": {}}}).to_string(),
        ],
        // --dry-run 已退役：与其他未知选项一样在解析期、任何写之前拒绝
        vec![
            "--key".into(),
            "aliases.shell".into(),
            "--value".into(),
            json!({"revision": good_revision, "value": {"aliases": {}}}).to_string(),
            "--dry-run".into(),
        ],
    ];
    for args in cases {
        let env = execute(&ctx, &request_owned("sources:set", args.clone()));
        assert!(!env.ok, "args {args:?} 应失败");
        assert_eq!(env.errors[0].code, "SOURCE_ERROR", "args {args:?}");
    }

    // 疑似凭据：拒绝且错误不回显正文
    let secret = json!({"revision": good_revision, "value": alias_value(&secret_body)}).to_string();
    let env = execute(
        &ctx,
        &request(
            "sources:set",
            &["--key", "aliases.shell", "--value", &secret],
        ),
    );
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "SOURCE_ERROR");
    assert!(
        !env.errors[0].message.contains("sk-"),
        "错误不得回显凭据正文: {}",
        env.errors[0].message
    );

    // 零写：未创建 DB/表/operation_logs，助手目标文件不存在
    assert!(!store::db_path(&ctx).exists());
    assert!(!ctx.home_dir.exists());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn existing_db_without_table_reads_defaults_with_zero_write() {
    let (base, ctx) = temp_ctx("existing-db");
    let conn = store::open_write(&ctx).unwrap();
    drop(conn);
    let db = store::db_path(&ctx);
    let bytes_before = std::fs::read(&db).unwrap();

    let env = execute(&ctx, &request("sources:get", &["--key", "aliases"]));
    assert!(env.ok, "{:?}", env.errors);
    let items = env.data["sources"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let serialized = serde_json::to_string(&items[0]["value"]).unwrap();
    assert_eq!(items[0]["revision"], sha256_hex(&serialized));

    // --dry-run 已退役：与过期 revision 一样在打开写连接前被拒绝
    let rev = items[0]["revision"].as_str().unwrap().to_string();
    let wrapper = json!({"revision": rev, "value": alias_value("echo x")}).to_string();
    let dry = execute(
        &ctx,
        &request(
            "sources:set",
            &["--key", "aliases.shell", "--value", &wrapper, "--dry-run"],
        ),
    );
    assert!(!dry.ok, "--dry-run 已退役，必须拒绝: {:?}", dry.data);
    assert_eq!(dry.errors[0].code, "SOURCE_ERROR");

    // 过期 revision 在打开写连接前即被拒绝
    let stale = json!({"revision": "0".repeat(64), "value": alias_value("echo x")}).to_string();
    let err = execute(
        &ctx,
        &request(
            "sources:set",
            &["--key", "aliases.shell", "--value", &stale],
        ),
    );
    assert!(!err.ok);
    assert_eq!(err.errors[0].code, "SOURCE_ERROR");

    assert_eq!(
        std::fs::read(&db).unwrap(),
        bytes_before,
        "只读路径 DB 字节不变"
    );
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let has_table: bool = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='alias_sources'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
        > 0;
    assert!(!has_table, "读路径不得创建 alias_sources 表");
    drop(conn);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn positional_body_marker_never_leaks_into_envelope() {
    let (base, ctx) = temp_ctx("no-body-leak");
    let marker = format!("LEAKMARKER-{}-唯一占位正文标记", std::process::id());
    let revision = alias_revision(&ctx);
    let wrapper = json!({"revision": revision, "value": alias_value("echo hi")}).to_string();

    // 真实 set：正文标记被误传为位置参数
    let env = execute(
        &ctx,
        &request_owned(
            "sources:set",
            vec![
                "--key".into(),
                "aliases.shell".into(),
                "--value".into(),
                wrapper.clone(),
                marker.clone(),
            ],
        ),
    );
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "SOURCE_ERROR");
    assert_eq!(env.errors[0].message, "sources 参数错误：不接受位置参数");
    assert!(!env.errors[0].message.contains(&marker));

    // 同类解析错误同样不得回显参数值：未知选项与重复 flag
    for extra in [
        vec![format!("--{marker}")],
        vec![format!("--{marker}"), format!("--{marker}")],
    ] {
        let mut args = vec![
            "--key".into(),
            "aliases.shell".into(),
            "--value".into(),
            wrapper.clone(),
        ];
        args.extend(extra);
        let env = execute(&ctx, &request_owned("sources:set", args));
        assert!(!env.ok);
        assert_eq!(env.errors[0].code, "SOURCE_ERROR");
        assert!(
            !env.errors[0].message.contains(&marker),
            "错误消息不得回显参数值: {}",
            env.errors[0].message
        );
    }

    // 诊断日志已退役：错误路径不得在数据目录留下日志文件
    assert!(!ctx.data_dir.join("logs").exists(), "不应再写诊断日志目录");

    // SQLite / 助手目标文件 / operation_logs 零写
    assert!(!store::db_path(&ctx).exists());
    assert!(!ctx.home_dir.exists());
    let _ = std::fs::remove_dir_all(base);
}

/// Hooks 来源已退役：生效 hooks 以各助手的实际配置为准（hooks 域直接编辑），旧
/// hook_sources 只作「旧版已保存的 hooks（未生效）」建议。hooks 前缀与精确键在 get/set
/// 上一律拒绝，且零写、不回显正文。
#[test]
fn hooks_source_keys_are_retired_with_zero_writes() {
    let (base, ctx) = temp_ctx("hooks-retired");
    let marker = "HOOK-RETIRED-MARKER-51e0";
    let wrapper = json!({
        "revision": "0".repeat(64),
        "value": {"hooks": {"Stop": [{"hooks": [{"type": "command", "command": marker}]}]}}
    })
    .to_string();
    for key in ["hooks", "hooks.claude", "hooks.codex"] {
        let env = execute(&ctx, &request("sources:get", &["--key", key]));
        assert!(!env.ok, "{key} 已退役");
        assert_eq!(env.errors[0].code, "SOURCE_ERROR");
        assert!(env.errors[0].message.contains("Hooks 来源已退役"), "{key}");
        let env = execute(
            &ctx,
            &request("sources:set", &["--key", key, "--value", &wrapper]),
        );
        assert!(!env.ok, "{key} 已退役");
        assert_eq!(env.errors[0].code, "SOURCE_ERROR");
        assert!(env.errors[0].message.contains("Hooks 来源已退役"), "{key}");
        assert!(!serde_json::to_string(&env).unwrap().contains(marker));
    }
    assert!(!ctx.data_dir.exists(), "拒绝不得创建数据目录");
    assert!(!ctx.home_dir.exists(), "拒绝不得写助手文件");

    // 已有库：同样拒绝，库字节不变，不会建出 hook_sources 表
    drop(store::open_write(&ctx).unwrap());
    let db = store::db_path(&ctx);
    let before = std::fs::read(&db).unwrap();
    let env = execute(
        &ctx,
        &request(
            "sources:set",
            &["--key", "hooks.claude", "--value", &wrapper],
        ),
    );
    assert!(!env.ok);
    assert_eq!(std::fs::read(&db).unwrap(), before);
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let has_table: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='hook_sources'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(has_table, 0);
    let _ = std::fs::remove_dir_all(base);
}
