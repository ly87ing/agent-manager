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

const DEFAULT_SHARED: &str = include_str!("../../agent-rule/rules/src/shared/core.md");
const DEFAULT_CLAUDE: &str = include_str!("../../agent-rule/rules/src/agents/claude/adapter.md");
const DEFAULT_CODEX: &str = include_str!("../../agent-rule/rules/src/agents/codex/adapter.md");

fn assert_source_item_shape(item: &Value, key: &str, value: &str) {
    let obj = item.as_object().unwrap();
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["format", "key", "label", "revision", "value"],
        "sources[] 项严格为 key/label/value/revision/format"
    );
    assert_eq!(item["key"], key);
    assert!(item["label"].is_string());
    assert!(!item["label"].as_str().unwrap().is_empty());
    assert_eq!(item["value"], value);
    assert_eq!(item["revision"], sha256_hex(value));
    assert_eq!(item["format"], "text");
}

#[test]
fn get_returns_compiled_defaults_for_prefix_and_exact_keys() {
    let (base, ctx) = temp_ctx("defaults");
    let env = execute(&ctx, &request("sources:get", &["--key", "rules"]));
    assert!(env.ok, "{:?}", env.errors);
    let items = env.data["sources"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_source_item_shape(&items[0], "rules.shared", DEFAULT_SHARED);
    assert_source_item_shape(&items[1], "rules.claude", DEFAULT_CLAUDE);
    assert_source_item_shape(&items[2], "rules.codex", DEFAULT_CODEX);
    assert_eq!(items[0]["label"], "共享核心规则");
    assert_eq!(items[1]["label"], "Claude 专用适配");
    assert_eq!(items[2]["label"], "Codex 专用适配");

    for (key, default) in [
        ("rules.shared", DEFAULT_SHARED),
        ("rules.claude", DEFAULT_CLAUDE),
        ("rules.codex", DEFAULT_CODEX),
    ] {
        let env = execute(&ctx, &request("sources:get", &["--key", key]));
        assert!(env.ok, "{key}: {:?}", env.errors);
        let items = env.data["sources"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_source_item_shape(&items[0], key, default);
    }
    assert!(!ctx.data_dir.exists(), "sources:get 不得创建数据目录");
    assert!(!ctx.home_dir.exists());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn get_rejects_nonconforming_args_with_source_error_and_zero_writes() {
    let (base, ctx) = temp_ctx("get-strict");
    let cases: Vec<Vec<&str>> = vec![
        vec![],
        vec!["--key"],
        vec!["--key", "--value"],
        vec!["--key", "rules.nope"],
        vec!["--key", "RULES"],
        vec!["--key", "rules", "--key", "rules"],
        vec!["--key", "rules", "positional"],
        vec!["--key", "rules", "--offline"],
        vec!["--key", "rules", "--value", "{}"],
        vec!["--key", "rules", "--dry-run"],
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
    let current = execute(&ctx, &request("sources:get", &["--key", "rules.shared"]));
    let revision = current.data["sources"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    let candidate = "新的共享规则正文\n第二行\n";
    let wrapper = json!({ "revision": revision, "value": candidate }).to_string();
    let env = execute(
        &ctx,
        &request(
            "sources:set",
            &["--key", "rules.shared", "--value", &wrapper, "--dry-run"],
        ),
    );
    assert!(!env.ok, "--dry-run 已退役，必须拒绝: {:?}", env.data);
    assert_eq!(env.errors[0].code, "SOURCE_ERROR");
    // 零目标写：无 DB/表/apply lock/助手目标文件（失败写动作的诊断日志按统一
    // 策略写 data_dir，不属于目标状态）
    assert!(!store::db_path(&ctx).exists());
    assert!(!ctx.home_dir.exists());
    assert!(!ctx.data_dir.join("agent-manager.apply.lock").exists());
    let after = execute(&ctx, &request("sources:get", &["--key", "rules.shared"]));
    assert_eq!(after.data["sources"][0]["value"], DEFAULT_SHARED);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn set_rejects_invalid_secret_nul_and_stale_inputs_without_writes() {
    let (base, ctx) = temp_ctx("set-invalid");
    let current = execute(&ctx, &request("sources:get", &["--key", "rules.shared"]));
    let good_revision = current.data["sources"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    let secret_body = format!("sk-{}", "a".repeat(24));

    let cases: Vec<Vec<String>> = vec![
        // 前缀/未知 key
        vec!["--key".into(), "rules".into()],
        vec![
            "--key".into(),
            "rules.nope".into(),
            "--value".into(),
            json!({"revision": good_revision, "value": "x"}).to_string(),
        ],
        // 缺 --value / 缺值 / 位置参数 / 未知 flag / 重复 flag
        vec!["--key".into(), "rules.shared".into()],
        vec!["--key".into(), "rules.shared".into(), "--value".into()],
        vec!["positional".into()],
        vec![
            "--key".into(),
            "rules.shared".into(),
            "--value".into(),
            "{}".into(),
            "--offline".into(),
        ],
        vec![
            "--key".into(),
            "rules.shared".into(),
            "--key".into(),
            "rules.claude".into(),
            "--value".into(),
            "{}".into(),
        ],
        // 非法 wrapper
        vec![
            "--key".into(),
            "rules.shared".into(),
            "--value".into(),
            "{not json".into(),
        ],
        vec![
            "--key".into(),
            "rules.shared".into(),
            "--value".into(),
            "true".into(),
        ],
        vec![
            "--key".into(),
            "rules.shared".into(),
            "--value".into(),
            "[]".into(),
        ],
        vec![
            "--key".into(),
            "rules.shared".into(),
            "--value".into(),
            json!({"revision": good_revision, "value": "x", "extra": 1}).to_string(),
        ],
        vec![
            "--key".into(),
            "rules.shared".into(),
            "--value".into(),
            json!({"value": "x"}).to_string(),
        ],
        vec![
            "--key".into(),
            "rules.shared".into(),
            "--value".into(),
            json!({"revision": good_revision}).to_string(),
        ],
        vec![
            "--key".into(),
            "rules.shared".into(),
            "--value".into(),
            json!({"revision": 1, "value": "x"}).to_string(),
        ],
        vec![
            "--key".into(),
            "rules.shared".into(),
            "--value".into(),
            json!({"revision": good_revision, "value": 1}).to_string(),
        ],
        // NUL 正文
        vec![
            "--key".into(),
            "rules.shared".into(),
            "--value".into(),
            json!({"revision": good_revision, "value": "a\0b"}).to_string(),
        ],
        // 过期 revision
        vec![
            "--key".into(),
            "rules.shared".into(),
            "--value".into(),
            json!({"revision": "0".repeat(64), "value": "x"}).to_string(),
        ],
        // --dry-run 已退役：与其他未知选项一样在解析期、任何写之前拒绝
        vec![
            "--key".into(),
            "rules.shared".into(),
            "--value".into(),
            json!({"revision": good_revision, "value": "x"}).to_string(),
            "--dry-run".into(),
        ],
    ];
    for args in cases {
        let env = execute(&ctx, &request_owned("sources:set", args.clone()));
        assert!(!env.ok, "args {args:?} 应失败");
        assert_eq!(env.errors[0].code, "SOURCE_ERROR", "args {args:?}");
    }

    // 疑似凭据：拒绝且错误不回显正文
    let secret = json!({"revision": good_revision, "value": secret_body}).to_string();
    let env = execute(
        &ctx,
        &request(
            "sources:set",
            &["--key", "rules.shared", "--value", &secret],
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
    assert!(!ctx.home_dir.join(".claude/CLAUDE.md").exists());
    assert!(!ctx.home_dir.join(".codex/AGENTS.md").exists());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn real_set_roundtrip_revision_cas_and_audit_without_body() {
    let (base, ctx) = temp_ctx("set-roundtrip");
    let claude_md = ctx.home_dir.join(".claude/CLAUDE.md");
    let codex_md = ctx.home_dir.join(".codex/AGENTS.md");
    std::fs::create_dir_all(claude_md.parent().unwrap()).unwrap();
    std::fs::create_dir_all(codex_md.parent().unwrap()).unwrap();
    std::fs::write(&claude_md, b"# claude sentinel\n").unwrap();
    std::fs::write(&codex_md, b"# codex sentinel\n").unwrap();

    let env = execute(&ctx, &request("sources:get", &["--key", "rules.shared"]));
    let rev0 = env.data["sources"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(rev0, sha256_hex(DEFAULT_SHARED));

    let v1 = "团队规则 v1：显式假设。\n";
    let wrapper = json!({"revision": rev0, "value": v1}).to_string();
    let env = execute(
        &ctx,
        &request(
            "sources:set",
            &["--key", "rules.shared", "--value", &wrapper],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_source_item_shape(&env.data["sources"][0], "rules.shared", v1);
    let rev1 = sha256_hex(v1);
    assert_ne!(rev0, rev1);

    let env = execute(&ctx, &request("sources:get", &["--key", "rules"]));
    let items = env.data["sources"].as_array().unwrap();
    assert_eq!(items[0]["value"], v1);
    assert_eq!(items[0]["revision"], rev1);
    assert_eq!(items[1]["value"], DEFAULT_CLAUDE);
    assert_eq!(items[2]["value"], DEFAULT_CODEX);

    // 过期 revision CAS 拒绝，已存内容不变且错误不回显候选正文
    let stale = json!({"revision": rev0, "value": "不应写入"}).to_string();
    let env = execute(
        &ctx,
        &request("sources:set", &["--key", "rules.shared", "--value", &stale]),
    );
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "SOURCE_ERROR");
    assert!(!env.errors[0].message.contains("不应写入"));
    let env = execute(&ctx, &request("sources:get", &["--key", "rules.shared"]));
    assert_eq!(env.data["sources"][0]["value"], v1);

    // 当前 revision 再次保存成功
    let v2 = "团队规则 v2：最小改动。\n";
    let wrapper2 = json!({"revision": rev1, "value": v2}).to_string();
    let env = execute(
        &ctx,
        &request(
            "sources:set",
            &["--key", "rules.shared", "--value", &wrapper2],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
    let rev2 = sha256_hex(v2);
    let env = execute(&ctx, &request("sources:get", &["--key", "rules.shared"]));
    assert_eq!(env.data["sources"][0]["value"], v2);

    // 审计：只记 key 与新 revision，不记正文
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT target, detail_json FROM operation_logs
             WHERE operation_type='sources:set' ORDER BY created_at, rowid",
        )
        .unwrap();
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "rules.shared");
    assert_eq!(rows[1].0, "rules.shared");
    assert!(rows[0].1.contains(&rev1));
    assert!(rows[1].1.contains(&rev2));
    for (target, detail) in &rows {
        assert!(detail.contains("rules.shared"));
        assert!(!detail.contains("团队规则"), "审计不得包含正文: {detail}");
        assert!(!target.contains("团队规则"));
    }
    drop(stmt);
    drop(conn);

    // 助手目标文件字节不变；sources 不使用共享 apply lock
    assert_eq!(std::fs::read(&claude_md).unwrap(), b"# claude sentinel\n");
    assert_eq!(std::fs::read(&codex_md).unwrap(), b"# codex sentinel\n");
    assert!(!ctx.data_dir.join("agent-manager.apply.lock").exists());

    // policy 行为不受影响
    let env = execute(&ctx, &request("policy:get", &[]));
    assert!(env.ok);
    assert_eq!(
        env.data["effective"]["agents"]["definitions"]["codex"]["providers"]["native"]["upgrade"],
        "curl -fsSL https://chatgpt.com/codex/install.sh | sh"
    );
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn existing_db_without_table_reads_defaults_with_zero_write() {
    let (base, ctx) = temp_ctx("existing-db");
    let conn = store::open_write(&ctx).unwrap();
    drop(conn);
    let db = store::db_path(&ctx);
    let bytes_before = std::fs::read(&db).unwrap();

    let env = execute(&ctx, &request("sources:get", &["--key", "rules"]));
    assert!(env.ok, "{:?}", env.errors);
    let items = env.data["sources"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_source_item_shape(&items[0], "rules.shared", DEFAULT_SHARED);
    assert_source_item_shape(&items[1], "rules.claude", DEFAULT_CLAUDE);
    assert_source_item_shape(&items[2], "rules.codex", DEFAULT_CODEX);

    // --dry-run 已退役：与过期 revision 一样在打开写连接前被拒绝
    let rev = items[0]["revision"].as_str().unwrap().to_string();
    let wrapper = json!({"revision": rev, "value": "候选正文"}).to_string();
    let dry = execute(
        &ctx,
        &request(
            "sources:set",
            &["--key", "rules.shared", "--value", &wrapper, "--dry-run"],
        ),
    );
    assert!(!dry.ok, "--dry-run 已退役，必须拒绝: {:?}", dry.data);
    assert_eq!(dry.errors[0].code, "SOURCE_ERROR");

    // 过期 revision 在打开写连接前即被拒绝
    let stale = json!({"revision": "0".repeat(64), "value": "x"}).to_string();
    let err = execute(
        &ctx,
        &request("sources:set", &["--key", "rules.shared", "--value", &stale]),
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
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='rule_sources'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
        > 0;
    assert!(!has_table, "读路径不得创建 rule_sources 表");
    drop(conn);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn positional_body_marker_never_leaks_into_envelope_or_diagnostic_log() {
    let (base, ctx) = temp_ctx("no-body-leak");
    let marker = format!("LEAKMARKER-{}-唯一占位正文标记", std::process::id());
    let current = execute(&ctx, &request("sources:get", &["--key", "rules.shared"]));
    let revision = current.data["sources"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    let wrapper = json!({"revision": revision, "value": "合法候选正文"}).to_string();

    // 非 dry-run 的真实 set：正文标记被误传为位置参数
    let env = execute(
        &ctx,
        &request_owned(
            "sources:set",
            vec![
                "--key".into(),
                "rules.shared".into(),
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
            "rules.shared".into(),
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

    // 真实 set 失败会写诊断日志：日志文件必须存在且不含正文标记
    let logs_dir = ctx.data_dir.join("logs");
    let entries: Vec<_> = std::fs::read_dir(&logs_dir)
        .unwrap_or_else(|_| panic!("真实 set 错误应记录诊断日志: {}", logs_dir.display()))
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!entries.is_empty(), "应至少有一条诊断日志文件");
    for entry in entries {
        let content = std::fs::read_to_string(entry.path()).unwrap();
        assert!(
            !content.contains(&marker),
            "诊断日志不得包含正文标记: {}",
            entry.path().display()
        );
    }

    // SQLite / 助手目标文件 / operation_logs 零写
    assert!(!store::db_path(&ctx).exists());
    assert!(!ctx.home_dir.join(".claude/CLAUDE.md").exists());
    assert!(!ctx.home_dir.join(".codex/AGENTS.md").exists());
    let _ = std::fs::remove_dir_all(base);
}

// ---------- hooks 来源（hooks.claude / hooks.codex） ----------

fn canonicalize_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            let mut out = serde_json::Map::new();
            for (key, item) in entries {
                out.insert(key.clone(), canonicalize_value(item));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_value).collect()),
        other => other.clone(),
    }
}

fn canonical(json_text: &str) -> String {
    let value: Value = serde_json::from_str(json_text).unwrap();
    serde_json::to_string(&canonicalize_value(&value)).unwrap()
}

const RAW_DEFAULT_HOOKS_CLAUDE: &str = include_str!("../../config/claude-hooks.json");
const RAW_DEFAULT_HOOKS_CODEX: &str = include_str!("../../config/codex-hooks.json");

fn default_hook_canonical(raw: &str) -> String {
    let mut obj = serde_json::from_str::<Value>(raw)
        .unwrap()
        .as_object()
        .unwrap()
        .clone();
    obj.remove("settings_path");
    obj.remove("hooks_path");
    serde_json::to_string(&canonicalize_value(&Value::Object(obj))).unwrap()
}

#[test]
fn hooks_get_returns_defaults_for_prefix_and_exact_keys() {
    let (base, ctx) = temp_ctx("hooks-defaults");
    let env = execute(&ctx, &request("sources:get", &["--key", "hooks"]));
    assert!(env.ok, "{:?}", env.errors);
    let items = env.data["sources"].as_array().unwrap();
    assert_eq!(items.len(), 2, "hooks 前缀返回 Claude 后 Codex 两项");
    assert_eq!(items[0]["key"], "hooks.claude");
    assert_eq!(items[1]["key"], "hooks.codex");
    for item in items {
        assert_eq!(item["format"], "json");
        assert!(item["value"].is_object(), "hooks value 必须是 JSON 对象");
        let canonical = serde_json::to_string(&item["value"]).unwrap();
        assert_eq!(item["revision"], sha256_hex(&canonical));
    }
    // 默认值去掉 settings_path/hooks_path
    assert!(items[0]["value"].get("settings_path").is_none());
    assert!(items[0]["value"].get("removed_signatures").is_some());
    assert!(items[1]["value"].get("hooks_path").is_none());
    assert_eq!(items[1]["value"], json!({ "hooks": {} }));

    for (key, raw) in [
        ("hooks.claude", RAW_DEFAULT_HOOKS_CLAUDE),
        ("hooks.codex", RAW_DEFAULT_HOOKS_CODEX),
    ] {
        let env = execute(&ctx, &request("sources:get", &["--key", key]));
        assert!(env.ok, "{key}: {:?}", env.errors);
        let items = env.data["sources"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["key"], key);
        let canonical = serde_json::to_string(&items[0]["value"]).unwrap();
        assert_eq!(canonical, default_hook_canonical(raw));
        assert_eq!(items[0]["revision"], sha256_hex(&canonical));
    }
    assert!(!ctx.data_dir.exists(), "sources:get 不得创建数据目录");
    assert!(!store::db_path(&ctx).exists());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn hooks_set_roundtrip_stale_cas_and_audit_without_body() {
    let (base, ctx) = temp_ctx("hooks-set");
    // 哨兵目标：sources:set 绝不触碰
    let settings = ctx.home_dir.join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(&settings, b"{\"user\": true}\n").unwrap();

    let env = execute(&ctx, &request("sources:get", &["--key", "hooks.claude"]));
    let rev0 = env.data["sources"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string();

    let v1 = json!({
        "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "echo hi"}]}]},
        "removed_signatures": []
    });
    // --dry-run 已退役：合法候选同样在任何写之前拒绝
    let dry = execute(
        &ctx,
        &request_owned(
            "sources:set",
            vec![
                "--key".into(),
                "hooks.claude".into(),
                "--value".into(),
                json!({"revision": rev0, "value": v1}).to_string(),
                "--dry-run".into(),
            ],
        ),
    );
    assert!(!dry.ok, "--dry-run 已退役，必须拒绝: {:?}", dry.data);
    assert_eq!(dry.errors[0].code, "SOURCE_ERROR");
    assert!(!store::db_path(&ctx).exists(), "--dry-run 拒绝零写");

    // 真实保存成功；value  canonical 化、revision 变化
    let env = execute(
        &ctx,
        &request_owned(
            "sources:set",
            vec![
                "--key".into(),
                "hooks.claude".into(),
                "--value".into(),
                json!({"revision": rev0, "value": v1}).to_string(),
            ],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
    let item = &env.data["sources"][0];
    assert_eq!(item["format"], "json");
    let rev1 = item["revision"].as_str().unwrap().to_string();
    assert_eq!(rev1, sha256_hex(&canonical(&v1.to_string())));
    assert_ne!(rev0, rev1);

    // stale CAS 拒绝且内容不变
    let stale = execute(
        &ctx,
        &request_owned(
            "sources:set",
            vec![
                "--key".into(),
                "hooks.claude".into(),
                "--value".into(),
                json!({"revision": rev0, "value": {"hooks": {}}}).to_string(),
            ],
        ),
    );
    assert!(!stale.ok);
    assert_eq!(stale.errors[0].code, "SOURCE_ERROR");
    let env = execute(&ctx, &request("sources:get", &["--key", "hooks.claude"]));
    assert_eq!(env.data["sources"][0]["revision"], rev1);

    // 审计只有 key/revision，无正文；不写目标文件、不取共享锁
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let detail: String = conn
        .query_row(
            "SELECT detail_json FROM operation_logs WHERE operation_type='sources:set' AND target='hooks.claude'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(detail.contains("hooks.claude"));
    assert!(detail.contains(&rev1));
    assert!(
        !detail.contains("echo hi"),
        "审计不得含 command 正文: {detail}"
    );
    drop(conn);
    assert_eq!(std::fs::read(&settings).unwrap(), b"{\"user\": true}\n");
    assert!(!ctx.data_dir.join("agent-manager.apply.lock").exists());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn hooks_set_rejects_invalid_schema_nul_secret_and_never_echoes_marker() {
    let (base, ctx) = temp_ctx("hooks-invalid");
    let marker = "HOOK-MARKER-9d2f-唯一命令标记";
    let env = execute(&ctx, &request("sources:get", &["--key", "hooks.codex"]));
    let rev = env.data["sources"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string();

    let valid_hooks = |command: &str| json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": command}]}]}});

    let cases: Vec<Value> = vec![
        // value 非对象
        json!({"revision": rev, "value": "text"}),
        json!({"revision": rev, "value": []}),
        // 多余字段
        json!({"revision": rev, "value": {"hooks": {}, "extra": 1}}),
        // hooks 非对象 / 事件空数组 / entry 缺 hooks / type 错 / command 空
        json!({"revision": rev, "value": {"hooks": []}}),
        json!({"revision": rev, "value": {"hooks": {"SessionStart": []}}}),
        json!({"revision": rev, "value": {"hooks": {"SessionStart": [{}]}}}),
        json!({"revision": rev, "value": {"hooks": {"SessionStart": [{"hooks": [{"type": "other", "command": "x"}]}]}}}),
        json!({"revision": rev, "value": {"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": " "}]}]}}}),
        // matcher 类型错
        json!({"revision": rev, "value": {"hooks": {"SessionStart": [{"matcher": 1, "hooks": [{"type": "command", "command": "x"}]}]}}}),
        // NUL（JSON 转义）
        json!({"revision": rev, "value": {"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "a\0b"}]}]}}}),
        // 空/纯空白 event 名
        json!({"revision": rev, "value": {"hooks": {"": [{"hooks": [{"type": "command", "command": "x"}]}]}}}),
        json!({"revision": rev, "value": {"hooks": {"  ": [{"hooks": [{"type": "command", "command": "x"}]}]}}}),
        // event key 含 NUL
        json!({"revision": rev, "value": {"hooks": {"Ev\0x": [{"hooks": [{"type": "command", "command": "x"}]}]}}}),
    ];
    for wrapper in cases {
        let env = execute(
            &ctx,
            &request_owned(
                "sources:set",
                vec![
                    "--key".into(),
                    "hooks.codex".into(),
                    "--value".into(),
                    wrapper.to_string(),
                ],
            ),
        );
        assert!(!env.ok, "{wrapper} 应失败");
        assert_eq!(env.errors[0].code, "SOURCE_ERROR");
    }

    // claude 专属字段：codex 拒绝 removed_signatures
    let env = execute(
        &ctx,
        &request_owned(
            "sources:set",
            vec![
                "--key".into(),
                "hooks.codex".into(),
                "--value".into(),
                json!({"revision": rev, "value": {"hooks": {}, "removed_signatures": []}})
                    .to_string(),
            ],
        ),
    );
    assert!(!env.ok);

    // 空白 command_contains：拒绝（避免空白 contains 广泛误删）
    let rev_claude = execute(&ctx, &request("sources:get", &["--key", "hooks.claude"])).data
        ["sources"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    let env = execute(
        &ctx,
        &request_owned(
            "sources:set",
            vec![
                "--key".into(),
                "hooks.claude".into(),
                "--value".into(),
                json!({"revision": rev_claude, "value": {"hooks": {}, "removed_signatures": [{"event": "PreToolUse", "command_contains": "  "}]}}).to_string(),
            ],
        ),
    );
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "SOURCE_ERROR");

    // 疑似凭据 command：拒绝且不回显
    let secret_body = format!("sk-{}", "b".repeat(24));
    let env = execute(
        &ctx,
        &request_owned(
            "sources:set",
            vec![
                "--key".into(),
                "hooks.codex".into(),
                "--value".into(),
                json!({"revision": rev, "value": valid_hooks(&secret_body)}).to_string(),
            ],
        ),
    );
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "SOURCE_ERROR");
    assert!(!env.errors[0].message.contains("sk-"), "错误不得回显凭据");

    // marker command 的合法结构失败路径（stale revision）不回显 marker
    let env = execute(
        &ctx,
        &request_owned(
            "sources:set",
            vec![
                "--key".into(),
                "hooks.codex".into(),
                "--value".into(),
                json!({"revision": "0".repeat(64), "value": valid_hooks(marker)}).to_string(),
            ],
        ),
    );
    assert!(!env.ok);
    let serialized = serde_json::to_string(&env).unwrap();
    assert!(!serialized.contains(marker), "envelope 不得回显 marker");
    if let Some(log_file) = &env.log_file {
        let log_text = std::fs::read_to_string(log_file).unwrap();
        assert!(!log_text.contains(marker), "诊断日志不得回显 marker");
    }

    assert!(!store::db_path(&ctx).exists(), "全部失败路径零写");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn hooks_set_is_key_order_canonical_across_root_event_and_nested_objects() {
    let (base, ctx) = temp_ctx("hooks-canonical");
    let get = |ctx: &Context| {
        execute(ctx, &request("sources:get", &["--key", "hooks.claude"])).data["sources"][0].clone()
    };
    let rev0 = get(&ctx)["revision"].as_str().unwrap().to_string();

    // 语义相同但根级、event、entry/hook 嵌套对象 key 顺序不同
    let ordered = json!({
        "hooks": {"SessionStart": [{"hooks": [{"command": "echo a", "type": "command"}], "matcher": "Write"}]},
        "removed_signatures": [{"event": "PreToolUse", "command_contains": "old.sh", "matcher": "Edit"}]
    });
    let shuffled = json!({
        "removed_signatures": [{"matcher": "Edit", "command_contains": "old.sh", "event": "PreToolUse"}],
        "hooks": {"SessionStart": [{"matcher": "Write", "hooks": [{"type": "command", "command": "echo a"}]}]}
    });

    let env = execute(
        &ctx,
        &request_owned(
            "sources:set",
            vec![
                "--key".into(),
                "hooks.claude".into(),
                "--value".into(),
                json!({"revision": rev0, "value": ordered}).to_string(),
            ],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
    let item1 = env.data["sources"][0].clone();

    let env = execute(
        &ctx,
        &request_owned(
            "sources:set",
            vec![
                "--key".into(),
                "hooks.claude".into(),
                "--value".into(),
                json!({"revision": item1["revision"].as_str().unwrap(), "value": shuffled})
                    .to_string(),
            ],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
    let item2 = env.data["sources"][0].clone();

    // 两次保存的 value/revision 完全相同（canonical 后键序一致）
    assert_eq!(
        serde_json::to_string(&item1["value"]).unwrap(),
        serde_json::to_string(&item2["value"]).unwrap()
    );
    assert_eq!(item1["revision"], item2["revision"]);
    assert_eq!(
        serde_json::to_string(&get(&ctx)["value"]).unwrap(),
        serde_json::to_string(&item1["value"]).unwrap()
    );
    // 存储字节本身是字典序 canonical compact
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let stored: String = conn
        .query_row(
            "SELECT value_json FROM hook_sources WHERE key='hooks.claude'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let expected_canonical = "{\"hooks\":{\"SessionStart\":[{\"hooks\":[{\"command\":\"echo a\",\"type\":\"command\"}],\"matcher\":\"Write\"}]},\"removed_signatures\":[{\"command_contains\":\"old.sh\",\"event\":\"PreToolUse\",\"matcher\":\"Edit\"}]}";
    assert_eq!(stored, expected_canonical);
    assert_eq!(item1["revision"], sha256_hex(expected_canonical));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn hooks_defaults_are_canonical_and_revision_matches() {
    let (base, ctx) = temp_ctx("hooks-default-canonical");
    let env = execute(&ctx, &request("sources:get", &["--key", "hooks"]));
    assert!(env.ok, "{:?}", env.errors);
    for item in env.data["sources"].as_array().unwrap() {
        let serialized = serde_json::to_string(&item["value"]).unwrap();
        assert_eq!(item["revision"], sha256_hex(&serialized));
        // 键序即 canonical 字典序：序列化后应与排序重建结果一致
        let value = &item["value"];
        let sorted_keys = {
            let mut keys: Vec<&String> = value.as_object().unwrap().keys().collect();
            keys.sort();
            keys.into_iter().cloned().collect::<Vec<_>>()
        };
        let actual_keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        assert_eq!(actual_keys, sorted_keys, "默认来源键序必须已 canonical");
    }
    let _ = std::fs::remove_dir_all(base);
}
