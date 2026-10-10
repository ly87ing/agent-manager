use agent_manager_engine::{execute, store, Context, Request};
#[cfg(windows)]
mod common;

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
    #[cfg(windows)]
    common::private_fixture_root(&base);
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

/// 直接写库构造旧版服务库定义（绕过 policy:set 校验，覆盖历史畸形反例）。
fn persist_policy(ctx: &Context, policy: &Value) {
    let conn = store::open_write(ctx).unwrap();
    store::write_policy(&conn, policy).unwrap();
}

fn policy_with_definitions(definitions: Value) -> Value {
    let mut policy = store::builtin_policy();
    policy["mcp"]["definitions"] = definitions;
    policy
}

/// 各助手实际 MCP 配置（隔离 HOME）。
fn write_claude(ctx: &Context, servers: Value) {
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    std::fs::write(
        ctx.home_dir.join(".claude.json"),
        serde_json::to_string_pretty(&json!({ "mcpServers": servers })).unwrap(),
    )
    .unwrap();
}

fn write_codex(ctx: &Context, text: &str) {
    std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    std::fs::write(ctx.home_dir.join(".codex/config.toml"), text).unwrap();
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

// 占位符矩阵：来源是各助手实际条目里的引用与停用快照，停用的 Codex 条目同样计入
// （原先按服务库定义与库级开关过滤——库级开关已随「服务库」模型退役，停用条目重新
// 启用时照样要用这些变量）；确定性排序，只含名称/布尔；字面量不是变量、不列出；
// 哨兵值不出现在序列化 envelope；status 零写。
#[test]
fn placeholder_matrix_is_sorted_boolean_only_and_never_leaks_values() {
    let (base, ctx) = temp_ctx("matrix");
    let sentinel = "SENTINEL-SECRET-VALUE-4b2c";
    let literal = "LITERAL-VALUE-7d31";
    std::env::set_var("AM_TEST_RESOLVED_VAR", sentinel);
    std::env::remove_var("AM_TEST_UNRESOLVED_VAR");
    write_claude(
        &ctx,
        json!({
            "zeta-server": {"command": "z", "env": {"Z_KEY": "${AM_TEST_UNRESOLVED_VAR}"}},
            "alpha-server": {"command": "a", "env": {
                "B_KEY": "${AM_TEST_UNRESOLVED_VAR}",
                "A_KEY": "${AM_TEST_RESOLVED_VAR}",
                "PLAIN": literal
            }},
        }),
    );
    write_codex(
        &ctx,
        "[mcp_servers.disabled-server]\ncommand = \"d\"\nenabled = false\n[mcp_servers.disabled-server.env]\nD_KEY = \"${AM_TEST_RESOLVED_VAR}\"\n",
    );

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
            ("disabled-server", "D_KEY", "AM_TEST_RESOLVED_VAR", true),
            ("zeta-server", "Z_KEY", "AM_TEST_UNRESOLVED_VAR", false),
        ]
    );
    assert_eq!(element["observed"]["unresolved"], 2);
    assert_eq!(element["state"], "unsupported");
    assert_eq!(element["observed"]["ownership_conflict"], true);
    let serialized = serde_json::to_string(&serde_json::to_value(&env).unwrap()).unwrap();
    assert!(!serialized.contains(sentinel), "值不得外泄: {serialized}");
    assert!(
        !serialized.contains(literal),
        "字面量不得外泄: {serialized}"
    );
    assert!(!ctx.data_dir.exists(), "status 不得创建数据目录");
    std::env::remove_var("AM_TEST_RESOLVED_VAR");
    let _ = std::fs::remove_dir_all(&base);
}

// 历史畸形旧版定义（字面量 env 值 / 非对象 env）：原先 secrets:status 整体 fail-closed；
// 旧定义现在按迁移规则推导成「未分配的已保存定义」（字面量改成同名引用、不带明文），
// 未分配定义不计入变量行。不变的要求：字面量/路径不回显，读取零写。
#[test]
fn malformed_persisted_definitions_never_leak_and_status_is_zero_write() {
    let (base, ctx) = temp_ctx("malformed");
    let literal_marker = "LITERAL-SECRET-9e71";
    for definitions in [
        json!({"marker-server": {"command": "m", "env": {"MARKER_KEY": literal_marker}}}),
        json!({"bad-env": {"command": "m", "env": "not-an-object"}}),
    ] {
        persist_policy(&ctx, &policy_with_definitions(definitions));
        let db_before = std::fs::read(store::db_path(&ctx)).unwrap();
        let env = secrets_status(&ctx, &[]);
        assert!(env.ok, "{:?}", env.errors);
        let element = mcp_env_element(&env);
        assert_eq!(element["observed"]["variables"], json!([]));
        let serialized = serde_json::to_string(&serde_json::to_value(&env).unwrap()).unwrap();
        assert!(!serialized.contains(literal_marker), "字面量不得回显");
        assert!(
            !serialized.contains(&base.display().to_string()),
            "路径不得回显"
        );
        let db_after = std::fs::read(store::db_path(&ctx)).unwrap();
        assert_eq!(db_before, db_after, "读取不得改变数据库字节");
        assert!(!ctx.data_dir.join("logs").exists(), "读取不得写日志");
        assert!(!ctx.home_dir.exists(), "读取不得创建 HOME 下的文件");
    }
    let _ = std::fs::remove_dir_all(&base);
}

// 停用快照里的引用计入变量行（重新启用时要解析）；实际条目里的字面量不列、不外泄。
#[test]
fn disabled_snapshot_references_are_listed_and_literals_never_leak() {
    let (base, ctx) = temp_ctx("snapshot-refs");
    let literal_marker = "DISABLED-LITERAL-5a90";
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let source = base.join("private.env");
    std::fs::write(&source, "AM_TEST_ENABLED_VAR=dummy-source-value\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(
        execute(
            &ctx,
            &request("secrets:source", &["--path", source.to_str().unwrap()])
        )
        .ok
    );
    write_claude(
        &ctx,
        json!({
            "enabled-server": {"command": "e", "env": {"E_KEY": "${AM_TEST_ENABLED_VAR}", "LIT": literal_marker}},
            "snap-server": {"command": "s", "env": {"S_KEY": "${AM_TEST_ENABLED_VAR}"}},
        }),
    );
    let status = execute(&ctx, &request("mcp:status", &[]));
    let base_of = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:snap-server")
        .unwrap()["observed"]["agents"]["claude-code"]["base"]
        .as_str()
        .unwrap()
        .to_string();
    let disabled = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:snap-server",
                "--agent",
                "claude-code",
                "--action",
                "disable",
                "--base",
                &base_of,
            ],
        ),
    );
    assert!(disabled.ok, "{:?}", disabled.errors);

    let env = secrets_status(&ctx, &[]);
    assert!(env.ok, "{:?}", env.errors);
    let element = mcp_env_element(&env);
    let variables = element["observed"]["variables"].as_array().unwrap();
    let rows: Vec<(&str, &str)> = variables
        .iter()
        .map(|v| {
            (
                v["server"].as_str().unwrap(),
                v["env_key"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![("enabled-server", "E_KEY"), ("snap-server", "S_KEY")]
    );
    assert_eq!(element["observed"]["unresolved"], 0);
    assert_eq!(element["state"], "managed-ok");
    let serialized = serde_json::to_string(&serde_json::to_value(&env).unwrap()).unwrap();
    assert!(!serialized.contains(literal_marker), "字面量不得外泄");
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
    // 变量来自实际条目里的引用（原先来自服务库定义，该模型已退役）
    write_claude(
        &ctx,
        json!({"file-server": {"command": "x", "env": {"TOKEN": "${FILE_TOKEN}"}}}),
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

/// 接入配置与独立环境用到的凭据变量也列出来：变量名、有没有取值、用在哪里；不含取值，零写。
#[test]
fn provider_key_variables_list_their_connections_and_instances() {
    let (base, ctx) = temp_ctx("provider-keys");
    let mut policy = store::builtin_policy();
    policy["agents"]["definitions"]["claude-code"]["settings"] = json!({
        "profiles": {
            "glm": {"label": "智谱", "base_url": "https://open.bigmodel.cn/api/anthropic", "token_var": "AM_TEST_PK_GLM", "model": "glm-5.3"},
            "local": {"base_url": "http://127.0.0.1:8080", "model": "m"}
        }
    });
    policy["agents"]["definitions"]["cc-glm"] = json!({
        "display_name": "Claude · GLM", "bin": "claude",
        "profile": {"base_agent": "claude-code", "config_dir": "~/.claude-glm",
            "env": {"ANTHROPIC_BASE_URL": "https://open.bigmodel.cn/api/anthropic", "ANTHROPIC_AUTH_TOKEN": "${AM_TEST_PK_GLM}"}}
    });
    policy["agents"]["definitions"]["cx-kimi"] = json!({
        "display_name": "Codex · Kimi", "bin": "codex",
        "profile": {"base_agent": "codex", "config_dir": "~/.codex-kimi",
            "base_url": "https://api.moonshot.cn/v1", "env_key": "AM_TEST_PK_KIMI"}
    });
    persist_policy(&ctx, &policy);
    std::env::set_var("AM_TEST_PK_KIMI", "pk-secret-value");

    let db_before = std::fs::read(ctx.data_dir.join("native-manager.db")).unwrap();
    let env = secrets_status(&ctx, &[]);
    assert!(env.ok, "{:?}", env.errors);
    let keys = mcp_env_element(&env)["observed"]["provider_keys"].clone();
    assert_eq!(
        keys,
        json!([
            {"variable": "AM_TEST_PK_GLM", "resolved": false, "stored": false, "used_by": [
                {"kind": "connection", "agent": "claude-code", "name": "glm", "label": "智谱"},
                {"kind": "profile", "id": "cc-glm", "label": "Claude · GLM"}
            ]},
            {"variable": "AM_TEST_PK_KIMI", "resolved": true, "stored": false, "used_by": [
                {"kind": "profile", "id": "cx-kimi", "label": "Codex · Kimi"}
            ]}
        ])
    );
    assert!(!env.data.to_string().contains("pk-secret-value"));
    assert_eq!(
        std::fs::read(ctx.data_dir.join("native-manager.db")).unwrap(),
        db_before,
        "查询零写"
    );
    std::env::remove_var("AM_TEST_PK_KIMI");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
#[cfg(unix)] // 用到 Unix 权限位
fn 密钥文件可以就地新建_仅本人可读_已有的不覆盖_不能建在数据目录() {
    use std::os::unix::fs::PermissionsExt;
    let (base, ctx) = temp_ctx("create-source");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let path = "~/.config/agent-manager/secrets.env";
    let real = ctx.home_dir.join(".config/agent-manager/secrets.env");

    // 不带 --create：文件不存在就拒绝（原行为不变）
    let refused = execute(&ctx, &request("secrets:source", &["--path", path]));
    assert!(!refused.ok);
    assert!(!real.exists());

    let created = execute(
        &ctx,
        &request("secrets:source", &["--path", path, "--create"]),
    );
    assert!(created.ok, "{:?}", created.errors);
    assert_eq!(created.data["created"], true);
    let file_mode = std::fs::metadata(&real).unwrap().permissions().mode() & 0o777;
    let dir_mode = std::fs::metadata(real.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(file_mode, 0o600);
    assert_eq!(dir_mode, 0o700);

    // 写进去的密钥只在这个文件里；状态只报变量名
    let set = execute(
        &ctx,
        &request(
            "secrets:set",
            &["--var", "KIMI_API_KEY", "--value", "sk-created-file-value"],
        ),
    );
    assert!(set.ok, "{:?}", set.errors);
    let status = secrets_status(&ctx, &[]);
    let source = &mcp_env_element(&status)["observed"]["source"];
    assert_eq!(source["ready"], true);
    assert_eq!(source["names"], json!(["KIMI_API_KEY"]));
    assert!(!serde_json::to_string(&status.data)
        .unwrap()
        .contains("sk-created-file-value"));

    // 再带 --create：文件已存在，一个字节都不动
    let before = std::fs::read(&real).unwrap();
    let again = execute(
        &ctx,
        &request("secrets:source", &["--path", path, "--create"]),
    );
    assert!(again.ok, "{:?}", again.errors);
    assert_eq!(again.data["created"], false);
    assert_eq!(std::fs::read(&real).unwrap(), before);

    // 数据目录里、相对路径：拒绝，什么都不建
    let inside = ctx.data_dir.join("secrets.env");
    let refused = execute(
        &ctx,
        &request(
            "secrets:source",
            &["--path", inside.to_str().unwrap(), "--create"],
        ),
    );
    assert!(!refused.ok);
    assert!(!inside.exists());
    let refused = execute(
        &ctx,
        &request("secrets:source", &["--path", "secrets.env", "--create"]),
    );
    assert!(!refused.ok);
    assert!(
        !execute(
            &ctx,
            &request("secrets:source", &["--path", "", "--create"])
        )
        .ok
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn shared_account_secret_cannot_be_changed_or_applied() {
    let (base, ctx) = temp_ctx("shared-account-secret");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let source = base.join("private.env");
    std::fs::write(&source, "SHARED_ACCOUNT_SECRET=original-account-value\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(
        execute(
            &ctx,
            &request("secrets:source", &["--path", source.to_str().unwrap()])
        )
        .ok
    );
    let mut policy = store::effective_policy(&ctx).unwrap();
    for id in ["account-a", "account-b"] {
        policy["agents"]["definitions"][id] = json!({"bin":"claude","profile":{"base_agent":"claude-code","config_dir":format!("~/.{id}"),"env":{"ANTHROPIC_AUTH_TOKEN":"${SHARED_ACCOUNT_SECRET}"}}});
    }
    // An old unsafe registration must remain visible without being silently migrated.
    persist_policy(&ctx, &policy);
    let before = std::fs::read(&source).unwrap();
    let result = execute(
        &ctx,
        &request(
            "secrets:set",
            &[
                "--var",
                "SHARED_ACCOUNT_SECRET",
                "--value",
                "replacement-account-value",
            ],
        ),
    );
    assert!(!result.ok);
    assert_eq!(std::fs::read(&source).unwrap(), before);
    for id in ["account-a", "account-b"] {
        let result = execute(
            &ctx,
            &request(
                "profiles:apply",
                &["--only", &format!("profiles:{id}"), "--base", "missing"],
            ),
        );
        assert!(!result.ok);
        assert!(!ctx.home_dir.join(format!(".{id}/settings.json")).exists());
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("original-account-value"));
    }
    let status = secrets_status(&ctx, &[]);
    assert!(status.ok);
    assert_eq!(mcp_env_element(&status)["state"], "unsupported");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn bare_and_embedded_mcp_references_reserve_account_credential_ownership() {
    let (base, ctx) = temp_ctx("bare-embedded-owner");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let source = base.join("private.env");
    std::fs::write(&source, "ACCOUNT_MCP_ONLY=dummy-original\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(
        execute(
            &ctx,
            &request("secrets:source", &["--path", source.to_str().unwrap()])
        )
        .ok
    );
    write_claude(
        &ctx,
        json!({"embedded": {"command":"echo", "args":["prefix-${ACCOUNT_MCP_ONLY}-suffix"]}}),
    );
    write_codex(&ctx, "[mcp_servers.remote]\nurl = \"https://mcp.invalid\"\n[mcp_servers.remote.env_http_headers]\nX-API-Key = \"ACCOUNT_MCP_ONLY\"\n");
    let before = std::fs::read(&source).unwrap();
    let result = execute(
        &ctx,
        &request(
            "secrets:set",
            &["--var", "ACCOUNT_MCP_ONLY", "--value", "dummy-replacement"],
        ),
    );
    assert!(!result.ok);
    assert_eq!(std::fs::read(&source).unwrap(), before);
    let result = execute(&ctx, &request("secrets:source", &["--path", ""]));
    assert!(
        !result.ok,
        "a shared owner graph cannot change source for both accounts"
    );
    let status = secrets_status(&ctx, &[]);
    assert!(status.ok);
    assert_eq!(
        mcp_env_element(&status)["observed"]["ownership_conflict"],
        true
    );
    assert_eq!(mcp_env_element(&status)["state"], "unsupported");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn new_profile_existing_native_mcp_ref_cannot_take_another_accounts_credential() {
    let (base, ctx) = temp_ctx("candidate-native-owner");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let source = base.join("private.env");
    std::fs::write(
        &source,
        "CANDIDATE_A_KEY=dummy-a\nCANDIDATE_B_KEY=dummy-b\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(
        execute(
            &ctx,
            &request("secrets:source", &["--path", source.to_str().unwrap()])
        )
        .ok
    );
    write_claude(
        &ctx,
        json!({"owner-b": {"command":"echo", "env":{"TOKEN":"${CANDIDATE_B_KEY}"}}}),
    );
    let dir = ctx.home_dir.join(".claude-a");
    std::fs::create_dir_all(&dir).unwrap();
    let native = dir.join(".claude.json");
    std::fs::write(
        &native,
        "{\"mcpServers\":{\"copied-b\":{\"command\":\"echo\",\"args\":[\"${CANDIDATE_B_KEY}\"]}}}",
    )
    .unwrap();
    let native_before = std::fs::read(&native).unwrap();
    let main_before = std::fs::read(ctx.home_dir.join(".claude.json")).unwrap();
    let source_before = std::fs::read(&source).unwrap();
    let definition = json!({"bin":"claude", "profile":{"base_agent":"claude-code", "config_dir":"~/.claude-a", "env":{"ANTHROPIC_AUTH_TOKEN":"${CANDIDATE_A_KEY}"}}});
    let result = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.account-a",
                "--value",
                &definition.to_string(),
            ],
        ),
    );
    assert!(
        !result.ok,
        "candidate ownership must inspect implicit native MCP files"
    );
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("密钥变量")),
        "{:?}",
        result.errors
    );
    assert!(
        store::effective_policy(&ctx).unwrap()["agents"]["definitions"]
            .get("account-a")
            .is_none()
    );
    assert_eq!(std::fs::read(&native).unwrap(), native_before);
    assert_eq!(
        std::fs::read(ctx.home_dir.join(".claude.json")).unwrap(),
        main_before
    );
    assert_eq!(std::fs::read(&source).unwrap(), source_before);
    assert!(!dir.join("settings.json").exists());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn legacy_migration_rejects_shared_ownership_and_preserves_both_resources() {
    let (base, ctx) = temp_ctx("migration-shared-refusal");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let source = base.join("private.env");
    std::fs::write(&source, "MIGRATE_SHARED=\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(
        execute(
            &ctx,
            &request("secrets:source", &["--path", source.to_str().unwrap()])
        )
        .ok
    );
    write_claude(
        &ctx,
        json!({"owner-a":{"command":"echo","env":{"TOKEN":"${MIGRATE_SHARED}"}}}),
    );
    write_codex(&ctx, "[mcp_servers.owner-b]\nurl = \"https://mcp.invalid\"\nbearer_token_env_var = \"MIGRATE_SHARED\"\n");
    let conn = store::open_write(&ctx).unwrap();
    conn.execute_batch("CREATE TABLE secret_values(name TEXT PRIMARY KEY NOT NULL,value TEXT NOT NULL,updated_at TEXT NOT NULL);").unwrap();
    conn.execute(
        "INSERT INTO secret_values VALUES (?1,?2,'test')",
        rusqlite::params!["MIGRATE_SHARED", "dummy-legacy-value"],
    )
    .unwrap();
    drop(conn);
    let before = std::fs::read(&source).unwrap();
    let result = execute(&ctx, &request("secrets:migrate-legacy", &[]));
    assert!(!result.ok);
    assert_eq!(std::fs::read(&source).unwrap(), before);
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM secret_values", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    assert!(!serde_json::to_string(&result)
        .unwrap()
        .contains("dummy-legacy-value"));
    drop(conn);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn resolved_mcp_credential_remains_owned_after_reference_disappears() {
    let (base, ctx) = temp_ctx("persisted-mcp-owner");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let source = base.join("private.env");
    std::fs::write(&source, "PERSISTED_MCP_ONLY=dummy-first-account\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(
        execute(
            &ctx,
            &request("secrets:source", &["--path", source.to_str().unwrap()])
        )
        .ok
    );
    let spec = r#"{"command":"echo","env":{"TOKEN":"${PERSISTED_MCP_ONLY}"}}"#;
    let added = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:owned",
                "--agent",
                "claude-code",
                "--action",
                "add",
                "--spec",
                spec,
            ],
        ),
    );
    assert!(added.ok, "{:?}", added.errors);
    let main = ctx.home_dir.join(".claude.json");
    let main_before = std::fs::read(&main).unwrap();
    assert!(
        !String::from_utf8_lossy(&main_before).contains("${PERSISTED_MCP_ONLY}"),
        "reference is resolved in native file"
    );
    let other = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:stolen",
                "--agent",
                "codex",
                "--action",
                "add",
                "--spec",
                spec,
            ],
        ),
    );
    assert!(
        !other.ok,
        "resolved literals must retain their original account binding"
    );
    assert_eq!(std::fs::read(&main).unwrap(), main_before);
    assert!(!ctx.home_dir.join(".codex/config.toml").exists());
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let owner: String = conn
        .query_row(
            "SELECT account FROM secret_variable_owners WHERE name=?1",
            ["PERSISTED_MCP_ONLY"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(owner, "claude-code");
    assert!(!serde_json::to_string(&other)
        .unwrap()
        .contains("dummy-first-account"));
    drop(conn);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn source_registration_failure_removes_only_its_empty_created_file() {
    let (base, ctx) = temp_ctx("source-registration-rollback");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let conn = store::open_write(&ctx).unwrap();
    conn.execute_batch("CREATE TRIGGER deny_secret_source BEFORE INSERT ON app_settings WHEN NEW.key='secrets_file_path' BEGIN SELECT RAISE(ABORT,'injected source failure'); END;").unwrap();
    drop(conn);
    let target = ctx.home_dir.join("new-private.env");
    let result = execute(
        &ctx,
        &request(
            "secrets:source",
            &["--path", target.to_str().unwrap(), "--create"],
        ),
    );
    assert!(!result.ok);
    assert!(
        !target.exists(),
        "a failed registration must not leave an empty source file"
    );
    assert!(store::effective_settings(&ctx)
        .unwrap()
        .secrets_file_path
        .is_empty());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn removed_account_claim_cannot_be_reused_by_a_new_registration_incarnation() {
    let (base, ctx) = temp_ctx("retired-incarnation");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let source = base.join("private.env");
    std::fs::write(
        &source,
        "INCARNATION_OLD_KEY=dummy-old\nINCARNATION_NEW_KEY=dummy-new\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(
        execute(
            &ctx,
            &request("secrets:source", &["--path", source.to_str().unwrap()])
        )
        .ok
    );
    let definition = json!({"bin":"claude", "profile":{"base_agent":"claude-code", "config_dir":"~/.claude-incarnation", "env":{"ANTHROPIC_AUTH_TOKEN":"${INCARNATION_OLD_KEY}"}}, "_delete":false,"_replace":true});
    let register = |value: &Value| {
        execute(
            &ctx,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "agents.definitions.incarnation",
                    "--value",
                    &value.to_string(),
                ],
            ),
        )
    };
    let first = register(&definition);
    assert!(first.ok, "{:?}", first.errors);
    let first_generation = store::effective_policy(&ctx).unwrap()["agents"]["definitions"]
        ["incarnation"]["account_generation"]
        .clone();
    assert!(first_generation.is_string());
    let removed = register(&json!({"_delete":true}));
    assert!(removed.ok, "{:?}", removed.errors);
    let before = std::fs::read(&source).unwrap();
    let reused = register(&definition);
    assert!(
        !reused.ok,
        "a new registration cannot reuse the retired account credential claim"
    );
    assert!(
        reused
            .errors
            .iter()
            .any(|error| error.message.contains("退役") || error.message.contains("密钥变量")),
        "{:?}",
        reused.errors
    );
    assert_eq!(std::fs::read(&source).unwrap(), before);
    let retired_set = execute(
        &ctx,
        &request(
            "secrets:set",
            &[
                "--var",
                "INCARNATION_OLD_KEY",
                "--value",
                "dummy-replacement",
            ],
        ),
    );
    assert!(
        !retired_set.ok,
        "retired credential values cannot be changed for a new identity"
    );
    assert_eq!(std::fs::read(&source).unwrap(), before);
    let mut fresh = definition;
    fresh["profile"]["env"]["ANTHROPIC_AUTH_TOKEN"] = json!("${INCARNATION_NEW_KEY}");
    let recreated = register(&fresh);
    assert!(
        recreated.ok,
        "a fresh explicit account variable remains available: {:?}",
        recreated.errors
    );
    assert_ne!(
        store::effective_policy(&ctx).unwrap()["agents"]["definitions"]["incarnation"]
            ["account_generation"],
        first_generation
    );
    let next_source = base.join("next-private.env");
    std::fs::write(&next_source, "INCARNATION_NEW_KEY=dummy-next-source\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&next_source, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let source_change = execute(
        &ctx,
        &request("secrets:source", &["--path", next_source.to_str().unwrap()]),
    );
    assert!(
        source_change.ok,
        "unrelated retired claim must not block a live account source: {:?}",
        source_change.errors
    );
    let live_set = execute(
        &ctx,
        &request(
            "secrets:set",
            &[
                "--var",
                "INCARNATION_NEW_KEY",
                "--value",
                "dummy-live-updated",
            ],
        ),
    );
    assert!(live_set.ok, "{:?}", live_set.errors);
    assert_eq!(std::fs::read(&source).unwrap(), before);
    assert!(std::fs::read_to_string(&next_source)
        .unwrap()
        .contains("dummy-live-updated"));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn explicit_unchanged_legacy_mcp_account_save_initializes_reference_generation() {
    let (base, ctx) = temp_ctx("legacy-mcp-generation");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let source = base.join("private.env");
    std::fs::write(&source, "LEGACY_MCP_ACCOUNT_KEY=dummy-legacy-mcp-value\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(
        execute(
            &ctx,
            &request("secrets:source", &["--path", source.to_str().unwrap()])
        )
        .ok
    );
    let definition =
        json!({"bin":"opencode","mcp_config":{"path":"~/.mcp-account.json","format":"opencode"}});
    let mut policy = store::effective_policy(&ctx).unwrap();
    policy["agents"]["definitions"]["mcp-work"] = definition.clone();
    persist_policy(&ctx, &policy);
    let actual = ctx.home_dir.join(".mcp-account.json");
    std::fs::write(&actual, json!({"mcp":{"owned":{"type":"local","command":["echo"],"environment":{"TOKEN":"${LEGACY_MCP_ACCOUNT_KEY}"}}}}).to_string()).unwrap();
    let before = std::fs::read(&actual).unwrap();
    let source_before = std::fs::read(&source).unwrap();
    let saved = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.mcp-work",
                "--value",
                &definition.to_string(),
            ],
        ),
    );
    assert!(saved.ok, "{:?}", saved.errors);
    assert!(
        store::effective_policy(&ctx).unwrap()["agents"]["definitions"]["mcp-work"]
            ["account_generation"]
            .is_string()
    );
    assert_eq!(std::fs::read(&actual).unwrap(), before);
    assert_eq!(std::fs::read(&source).unwrap(), source_before);
    let changed = execute(
        &ctx,
        &request(
            "secrets:set",
            &[
                "--var",
                "LEGACY_MCP_ACCOUNT_KEY",
                "--value",
                "dummy-explicit-update",
            ],
        ),
    );
    assert!(changed.ok, "{:?}", changed.errors);
    assert_eq!(std::fs::read(&actual).unwrap(), before);
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let owner: String = conn
        .query_row(
            "SELECT account FROM secret_variable_owners WHERE name='LEGACY_MCP_ACCOUNT_KEY'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(owner, "mcp-work");
    drop(conn);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn removed_account_saved_refs_stay_retired_without_blocking_live_source_or_migration() {
    let (base, ctx) = temp_ctx("retired-snapshot-live-source");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let source = base.join("private.env");
    std::fs::write(
        &source,
        "RETIRED_SNAPSHOT_A=dummy-a\nLIVE_SNAPSHOT_B=dummy-b\n",
    )
    .unwrap();
    let next = base.join("next.env");
    std::fs::write(&next, "LIVE_SNAPSHOT_B=dummy-b\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [&source, &next] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
    assert!(
        execute(
            &ctx,
            &request("secrets:source", &["--path", source.to_str().unwrap()])
        )
        .ok
    );
    let register = |id: &str, raw: Value| {
        execute(
            &ctx,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    &format!("agents.definitions.{id}"),
                    "--value",
                    &raw.to_string(),
                ],
            ),
        )
    };
    for (id, key) in [
        ("snapshot-a", "RETIRED_SNAPSHOT_A"),
        ("snapshot-b", "LIVE_SNAPSHOT_B"),
    ] {
        let saved = register(
            id,
            json!({"bin":"claude", "profile":{"base_agent":"claude-code","config_dir":format!("~/.{id}"),"env":{"ANTHROPIC_AUTH_TOKEN":format!("${{{key}}}")}}}),
        );
        assert!(saved.ok, "{:?}", saved.errors);
    }
    let conn = store::open_write(&ctx).unwrap();
    conn.execute_batch("CREATE TABLE mcp_saved(server TEXT NOT NULL,agent TEXT NOT NULL,entry_json TEXT NOT NULL,resolve_json TEXT NOT NULL,origin TEXT NOT NULL,updated_at TEXT NOT NULL,PRIMARY KEY(server,agent));").unwrap();
    conn.execute("INSERT INTO mcp_saved VALUES ('old','snapshot-a',?1,'{\"env\":[],\"headers\":[]}','disabled','test')", [json!({"command":"echo","env":{"TOKEN":"${RETIRED_SNAPSHOT_A}"}}).to_string()]).unwrap();
    drop(conn);
    let deleted = register("snapshot-a", json!({"_delete":true}));
    assert!(deleted.ok, "{:?}", deleted.errors);
    let before = std::fs::read(&source).unwrap();
    let changed = execute(
        &ctx,
        &request("secrets:source", &["--path", next.to_str().unwrap()]),
    );
    assert!(changed.ok, "{:?}", changed.errors);
    let retired = execute(
        &ctx,
        &request(
            "secrets:set",
            &["--var", "RETIRED_SNAPSHOT_A", "--value", "dummy-reassigned"],
        ),
    );
    assert!(!retired.ok, "deleted snapshot credentials remain reserved");
    let conn = store::open_write(&ctx).unwrap();
    conn.execute_batch("CREATE TABLE secret_values(name TEXT PRIMARY KEY NOT NULL,value TEXT NOT NULL,updated_at TEXT NOT NULL);").unwrap();
    conn.execute(
        "INSERT INTO secret_values VALUES ('LIVE_SNAPSHOT_B','dummy-b','test')",
        [],
    )
    .unwrap();
    drop(conn);
    let migrated = execute(&ctx, &request("secrets:migrate-legacy", &[]));
    assert!(migrated.ok, "{:?}", migrated.errors);
    assert_eq!(std::fs::read(&source).unwrap(), before);
    assert_eq!(std::fs::read(&next).unwrap(), b"LIVE_SNAPSHOT_B=dummy-b\n");
    assert!(!ctx.home_dir.join(".snapshot-a/settings.json").exists());
    assert!(!ctx.home_dir.join(".snapshot-b/settings.json").exists());
    let _ = std::fs::remove_dir_all(base);
}
