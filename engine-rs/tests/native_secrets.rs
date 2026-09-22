use agent_manager_engine::{execute, store, Context, Request};
use serde_json::{json, Value};
use std::path::PathBuf;

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    std::env::remove_var("AGENT_MANAGER_LOCK_HELD");
    std::env::remove_var("AGENT_MANAGER_LOCK_PID");
    let base = std::env::temp_dir().join(format!(
        "am-native-secrets-test-{}-{}-{:x}",
        std::process::id(),
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
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
        run_id: "secrets-test-run".to_string(),
    }
}

fn secrets_status(ctx: &Context, args: &[&str]) -> agent_manager_engine::Envelope {
    execute(ctx, &request("secrets:status", args))
}

fn mcp_env_element(env: &agent_manager_engine::Envelope) -> Value {
    env.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "secrets:mcp-env")
        .unwrap_or_else(|| panic!("secrets:mcp-env 必须存在: {}", env.data))
        .clone()
}

/// 直接写库构造已启用 MCP 定义（绕过 policy:set 校验，覆盖历史畸形反例）。
fn persist_policy(ctx: &Context, policy: &Value) {
    let conn = store::open_write(ctx).unwrap();
    store::write_policy(&conn, policy).unwrap();
}

fn policy_with_definitions(definitions: Value) -> Value {
    let mut policy = store::builtin_policy();
    policy["mcp"]["definitions"] = definitions;
    policy
}

// fresh 隔离 Context：成功、空变量、offline 感知、零写。
#[test]
fn fresh_context_status_is_empty_offline_aware_and_zero_write() {
    let (base, ctx) = temp_ctx("fresh");
    let env = secrets_status(&ctx, &[]);
    assert!(env.ok, "{:?}", env.errors);
    let element = mcp_env_element(&env);
    assert_eq!(
        element["spec"]["sources"],
        json!(["external file", "process env"])
    );
    assert_eq!(element["policy"]["enabled"], true);
    assert_eq!(element["observed"]["variables"], json!([]));
    assert_eq!(element["observed"]["unresolved"], 0);
    assert_eq!(element["observed"]["source"]["configured"], false);
    assert_eq!(element["observed"]["legacy_database_values"], 0);
    assert_eq!(element["state"], "managed-ok");
    assert_eq!(element["freshness"]["state"], "not-applicable");
    assert_eq!(element["actions"], json!([]));
    // 不再有 secrets:repo 元素
    assert_eq!(env.data["elements"].as_array().unwrap().len(), 1);
    assert!(!ctx.data_dir.exists());
    assert!(!ctx.home_dir.exists());
    assert!(!store::db_path(&ctx).exists());
    let _ = std::fs::remove_dir_all(&base);
}

// 合法持久化占位符矩阵：确定性排序，只含名称/布尔；进程环境存在性决定
// resolved；哨兵值不出现在序列化 envelope。
#[test]
fn placeholder_matrix_is_sorted_boolean_only_and_never_leaks_values() {
    let (base, ctx) = temp_ctx("matrix");
    let sentinel = "SENTINEL-SECRET-VALUE-4b2c";
    std::env::set_var("AM_TEST_RESOLVED_VAR", sentinel);
    std::env::remove_var("AM_TEST_UNRESOLVED_VAR");
    let policy = policy_with_definitions(json!({
        "zeta-server": {"command": "z", "env": {"Z_KEY": "${AM_TEST_UNRESOLVED_VAR}"}},
        "alpha-server": {"command": "a", "env": {"B_KEY": "${AM_TEST_UNRESOLVED_VAR}", "A_KEY": "${AM_TEST_RESOLVED_VAR}"}},
        "disabled-server": {"enabled": false, "command": "d", "env": {"D_KEY": "${AM_TEST_RESOLVED_VAR}"}},
    }));
    // disabled-server 通过 mcp.<server>.enabled=false 开关关闭
    let mut policy = policy;
    policy["mcp"]["disabled-server"] = json!({"enabled": false});
    persist_policy(&ctx, &policy);

    let env = secrets_status(&ctx, &[]);
    assert!(env.ok, "{:?}", env.errors);
    let element = mcp_env_element(&env);
    let variables = element["observed"]["variables"].as_array().unwrap();
    let rows: Vec<(&str, &str, &str, bool)> = variables
        .iter()
        .map(|v| {
            (
                v["server"].as_str().unwrap(),
                v["env_key"].as_str().unwrap(),
                v["placeholder"].as_str().unwrap(),
                v["resolved"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("alpha-server", "A_KEY", "AM_TEST_RESOLVED_VAR", true),
            ("alpha-server", "B_KEY", "AM_TEST_UNRESOLVED_VAR", false),
            ("zeta-server", "Z_KEY", "AM_TEST_UNRESOLVED_VAR", false),
        ]
    );
    assert_eq!(element["observed"]["unresolved"], 2);
    assert_eq!(element["state"], "missing");
    let serialized = serde_json::to_string(&serde_json::to_value(&env).unwrap()).unwrap();
    assert!(!serialized.contains(sentinel), "值不得外泄: {serialized}");
    std::env::remove_var("AM_TEST_RESOLVED_VAR");
    let _ = std::fs::remove_dir_all(&base);
}

// 历史畸形已启用定义（字面量 env 值 / 非对象 env）：fail-closed 固定脱敏
// 错误，不回显字面量/server/key，零文件系统写入。
#[test]
fn malformed_persisted_definitions_fail_closed_redacted() {
    let (base, ctx) = temp_ctx("malformed");
    let literal_marker = "LITERAL-SECRET-9e71";
    let policy = policy_with_definitions(json!({
        "marker-server": {"command": "m", "env": {"MARKER_KEY": literal_marker}},
    }));
    persist_policy(&ctx, &policy);
    let db_before = std::fs::read(store::db_path(&ctx)).unwrap();
    let env = secrets_status(&ctx, &[]);
    assert!(!env.ok);
    assert_eq!(env.errors.len(), 1);
    assert_eq!(env.errors[0].code, "SECRET_ERROR");
    let serialized = serde_json::to_string(&serde_json::to_value(&env).unwrap()).unwrap();
    assert!(!serialized.contains(literal_marker), "字面量不得回显");
    assert!(!serialized.contains("marker-server"), "server 名不得回显");
    assert!(!serialized.contains("MARKER_KEY"), "env key 不得回显");
    assert!(
        !serialized.contains(&base.display().to_string()),
        "路径不得回显"
    );
    // 失败路径零写：DB 字节不变
    let db_after = std::fs::read(store::db_path(&ctx)).unwrap();
    assert_eq!(db_before, db_after, "失败状态不得改变数据库字节");
    assert!(!ctx.data_dir.join("logs").exists(), "失败状态不得写日志");

    let policy = policy_with_definitions(json!({
        "bad-env": {"command": "m", "env": "not-an-object"},
    }));
    persist_policy(&ctx, &policy);
    let db_before = std::fs::read(store::db_path(&ctx)).unwrap();
    let env = secrets_status(&ctx, &[]);
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "SECRET_ERROR");
    let db_after = std::fs::read(store::db_path(&ctx)).unwrap();
    assert_eq!(db_before, db_after);
    assert!(!ctx.home_dir.exists(), "失败路径零写");
    let _ = std::fs::remove_dir_all(&base);
}

// 参数反例：重复 --offline / 未知 flag / 位置参数 fail-closed、零写、不回显。
#[test]
fn status_args_are_strict_and_fail_closed() {
    let (base, ctx) = temp_ctx("args");
    for args in [
        &["--offline", "--offline"][..],
        &["--bogus-flag"][..],
        &["positional-marker"][..],
    ] {
        let env = secrets_status(&ctx, args);
        assert!(!env.ok, "{args:?} 必须失败");
        assert_eq!(env.errors.len(), 1);
        assert_eq!(env.errors[0].code, "SECRET_ERROR");
        assert!(env.data.as_object().unwrap().is_empty());
        let serialized = serde_json::to_string(&env.errors[0].message).unwrap();
        for token in args {
            if *token != "--offline" {
                assert!(!serialized.contains(token), "不得回显 '{token}'");
            }
        }
        assert!(!ctx.data_dir.exists(), "失败路径零写: {args:?}");
    }
    let env = secrets_status(&ctx, &["--offline"]);
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.mode.offline);
    assert!(!ctx.data_dir.exists());
    let _ = std::fs::remove_dir_all(&base);
}

// 已禁用定义即使含畸形 env 字面量也被忽略：status 成功且只输出已启用定义。
#[test]
fn disabled_definitions_with_malformed_env_are_ignored() {
    let (base, ctx) = temp_ctx("disabled-malformed");
    let literal_marker = "DISABLED-LITERAL-5a90";
    std::env::set_var("AM_TEST_ENABLED_VAR", "x");
    let mut policy = policy_with_definitions(json!({
        "enabled-server": {"command": "e", "env": {"E_KEY": "${AM_TEST_ENABLED_VAR}"}},
        "disabled-server": {"command": "d", "env": {"D_KEY": literal_marker}},
    }));
    policy["mcp"]["disabled-server"] = json!({"enabled": false});
    persist_policy(&ctx, &policy);
    let env = secrets_status(&ctx, &[]);
    assert!(env.ok, "{:?}", env.errors);
    let element = mcp_env_element(&env);
    let variables = element["observed"]["variables"].as_array().unwrap();
    assert_eq!(variables.len(), 1);
    assert_eq!(variables[0]["server"], "enabled-server");
    assert_eq!(variables[0]["env_key"], "E_KEY");
    assert_eq!(variables[0]["resolved"], true);
    assert_eq!(element["observed"]["unresolved"], 0);
    assert_eq!(element["state"], "managed-ok");
    let serialized = serde_json::to_string(&serde_json::to_value(&env).unwrap()).unwrap();
    assert!(
        !serialized.contains(literal_marker),
        "禁用定义字面量不得外泄"
    );
    std::env::remove_var("AM_TEST_ENABLED_VAR");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn external_file_is_explicit_private_and_never_leaks_values() {
    let (base, ctx) = temp_ctx("external-file");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let path = ctx.home_dir.join("private.env");
    const SECRET: &str = "external-secret-value-under-test";
    std::fs::write(&path, format!("FILE_TOKEN={SECRET}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let configured = execute(
        &ctx,
        &request("secrets:source", &["--path", path.to_str().unwrap()]),
    );
    assert!(configured.ok, "{:?}", configured.errors);
    persist_policy(
        &ctx,
        &policy_with_definitions(json!({
            "file-server": {"command": "x", "env": {"TOKEN": "${FILE_TOKEN}"}},
        })),
    );

    let status = secrets_status(&ctx, &[]);
    assert!(status.ok, "{:?}", status.errors);
    let element = mcp_env_element(&status);
    assert_eq!(element["observed"]["source"]["configured"], true);
    assert_eq!(element["observed"]["source"]["ready"], true);
    assert_eq!(element["observed"]["variables"][0]["resolved"], true);
    assert_eq!(element["observed"]["variables"][0]["stored"], true);
    assert!(!serde_json::to_string(&status).unwrap().contains(SECRET));

    let replaced = execute(
        &ctx,
        &request(
            "secrets:set",
            &["--var", "FILE_TOKEN", "--value", "replacement-under-test"],
        ),
    );
    assert!(replaced.ok, "{:?}", replaced.errors);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("replacement-under-test"));
    assert!(!written.contains(SECRET));
    assert!(!serde_json::to_string(&replaced)
        .unwrap()
        .contains("replacement-under-test"));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn legacy_database_values_require_explicit_migration_to_external_file() {
    let (base, ctx) = temp_ctx("legacy-migrate");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let path = ctx.home_dir.join("private.env");
    std::fs::write(&path, "LEGACY_TOKEN=\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(
        execute(
            &ctx,
            &request("secrets:source", &["--path", path.to_str().unwrap()]),
        )
        .ok
    );

    let conn = store::open_write(&ctx).unwrap();
    conn.execute_batch(
        "CREATE TABLE secret_values (name TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL, updated_at TEXT NOT NULL);",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO secret_values(name, value, updated_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![
            "LEGACY_TOKEN",
            "legacy-value-under-test",
            "2026-09-22T00:00:00Z"
        ],
    )
    .unwrap();
    drop(conn);

    let before = secrets_status(&ctx, &[]);
    assert_eq!(
        mcp_env_element(&before)["observed"]["legacy_database_values"],
        1
    );
    let migrated = execute(&ctx, &request("secrets:migrate-legacy", &[]));
    assert!(migrated.ok, "{:?}", migrated.errors);
    assert_eq!(migrated.data["migrated"], json!(["LEGACY_TOKEN"]));
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("legacy-value-under-test"));
    assert_eq!(
        mcp_env_element(&secrets_status(&ctx, &[]))["observed"]["legacy_database_values"],
        0
    );

    let _ = std::fs::remove_dir_all(&base);
}
