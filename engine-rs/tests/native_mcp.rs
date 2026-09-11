use agent_manager_engine::native_mcp::{
    apply::apply as direct_apply,
    render::render_target_content,
    rollback::{atomic_write_checked, safe_write_file, RollbackGuard},
    targets::extract_entry,
    Format,
};
use agent_manager_engine::{execute, store, Context, Request};
use fs2::FileExt;
use std::fs::OpenOptions;
use std::path::PathBuf;

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    let base =
        std::env::temp_dir().join(format!("am-native-mcp-test-{}-{}", std::process::id(), tag));
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
        run_id: "mcp-test-run".to_string(),
    }
}

fn request_with_run(command: &str, args: &[&str], run_id: &str) -> Request {
    Request {
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        run_id: run_id.to_string(),
    }
}

fn error_messages(env: &agent_manager_engine::Envelope) -> String {
    env.errors
        .iter()
        .map(|d| d.message.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}

fn load_contract_schema(rel_path: &str) -> serde_json::Value {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let p = manifest.join("../").join(rel_path);
    let s = std::fs::read_to_string(&p)
        .unwrap_or_else(|_| panic!("无法读取 schema 文件: {}", p.display()));
    serde_json::from_str(&s).unwrap()
}

fn validate_json_schema(
    val: &serde_json::Value,
    schema: &serde_json::Value,
    root_schema: &serde_json::Value,
) {
    if let Some(r) = schema.get("$ref").and_then(|v| v.as_str()) {
        if let Some(def_name) = r.strip_prefix("#/$defs/") {
            let def = root_schema
                .get("$defs")
                .and_then(|d| d.get(def_name))
                .unwrap_or_else(|| panic!("无法在 $defs 中找到引用 {}", def_name));
            validate_json_schema(val, def, root_schema);
            return;
        }
    }

    if let Some(type_val) = schema.get("type") {
        match type_val {
            serde_json::Value::String(t) => match t.as_str() {
                "object" => assert!(val.is_object(), "期望对象类型，实际为 {:?}", val),
                "array" => assert!(val.is_array(), "期望数组类型，实际为 {:?}", val),
                "string" => assert!(val.is_string(), "期望字符串类型，实际为 {:?}", val),
                "integer" => assert!(
                    val.is_i64() || (val.is_number() && !val.is_f64()),
                    "期望整数类型，实际为 {:?}",
                    val
                ),
                "boolean" => assert!(val.is_boolean(), "期望布尔类型，实际为 {:?}", val),
                "null" => assert!(val.is_null(), "期望 null 类型，实际为 {:?}", val),
                _ => {}
            },
            serde_json::Value::Array(types) => {
                let matched = types.iter().any(|t| match t.as_str().unwrap() {
                    "string" => val.is_string(),
                    "null" => val.is_null(),
                    "integer" => val.is_i64(),
                    "boolean" => val.is_boolean(),
                    "object" => val.is_object(),
                    "array" => val.is_array(),
                    _ => false,
                });
                assert!(matched, "实际值 {:?} 不符合联合类型 {:?}", val, types);
            }
            _ => {}
        }
    }

    if let Some(enums) = schema.get("enum").and_then(|v| v.as_array()) {
        assert!(
            enums.contains(val),
            "值 {:?} 不在 enum 列表 {:?} 中",
            val,
            enums
        );
    }

    if let Some(obj) = val.as_object() {
        if let Some(req_fields) = schema.get("required").and_then(|v| v.as_array()) {
            for req in req_fields {
                let req_key = req.as_str().unwrap();
                assert!(
                    obj.contains_key(req_key),
                    "缺失必填字段 '{}' (实际包含: {:?})",
                    req_key,
                    obj.keys().collect::<Vec<_>>()
                );
            }
        }
        if schema.get("additionalProperties").and_then(|v| v.as_bool()) == Some(false) {
            let props = schema.get("properties").and_then(|v| v.as_object());
            for key in obj.keys() {
                let allowed = props.map(|p| p.contains_key(key)).unwrap_or(false);
                assert!(allowed, "不允许的额外字段 '{}' 出现", key);
            }
        }
        if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
            for (prop_key, prop_schema) in props {
                if let Some(prop_val) = obj.get(prop_key) {
                    validate_json_schema(prop_val, prop_schema, root_schema);
                }
            }
        }
    }

    if let Some(arr) = val.as_array() {
        if let Some(items_schema) = schema.get("items") {
            for item in arr {
                validate_json_schema(item, items_schema, root_schema);
            }
        }
    }
}

#[test]
fn mcp_zero_write_guarantee_without_db() {
    let (root, ctx) = temp_ctx("zero-write");

    // 1. 无数据库、无任何目录时，mcp:status 正常执行且保证零写
    let res = execute(&ctx, &request("mcp:status", &[]));
    assert!(res.ok, "mcp:status 应成功返回: {:?}", res.errors);
    assert_eq!(res.data["domain"], "mcp");
    assert!(!ctx.data_dir.exists(), "mcp:status 不得创建 data_dir");
    assert!(!ctx.home_dir.exists(), "mcp:status 不得创建 home_dir");

    // 2. 无定义时执行 mcp:plan 预检拦截，依然保证零写
    let plan_fail = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:nonexistent",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(!plan_fail.ok, "不存在的服务定义应当拦截");
    assert!(!ctx.data_dir.exists(), "失败的 mcp:plan 不得创建 data_dir");

    // 写入一个合法的服务定义到 policy
    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.test-server",
                "--value",
                r#"{"command":"node","args":["server.js"],"agents":["claude-code","codex"]}"#,
            ],
        ),
    );
    assert!(set_res.ok, "写入定义应当成功: {:?}", set_res.errors);

    // 3. mcp:plan 应生成动作计划，但不得在目标文件或 mcp_bindings 写数据
    let plan_res = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:test-server",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(plan_res.ok, "mcp:plan 应当成功: {:?}", plan_res.errors);
    let actions = plan_res.data["actions"].as_array().unwrap();
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0]["op"], "write-file");
    assert_eq!(actions[1]["op"], "write-file");
    assert!(actions[1]["target"]
        .as_str()
        .unwrap()
        .starts_with("native-manager.db:mcp_bindings"));
    assert!(
        !ctx.home_dir.join(".claude.json").exists(),
        "mcp:plan 不得创建目标配置文件"
    );

    // 4. mcp:apply --dry-run 零写保证：不创建目标文件、不创建 lock 文件、不写入 mcp_bindings
    let dry_res = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:test-server",
                "--action",
                "add",
                "--agent",
                "claude",
                "--dry-run",
            ],
        ),
    );
    assert!(
        dry_res.ok,
        "mcp:apply --dry-run 应当成功: {:?}",
        dry_res.errors
    );
    assert_eq!(dry_res.data["dry_run"], true);
    assert!(
        !ctx.home_dir.join(".claude.json").exists(),
        "dry-run 不得创建目标配置文件"
    );
    assert!(
        !ctx.data_dir.join("agent-manager.apply.lock").exists(),
        "dry-run 不得创建 lock 文件"
    );

    // 检查 mcp_bindings 表不存在或没有任何行
    if store::db_path(&ctx).exists() {
        let conn = store::open_readonly(&ctx).unwrap().unwrap();
        let table_exists: bool = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='mcp_bindings'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);
        if table_exists {
            let row_count: i64 = conn
                .query_row("SELECT count(*) FROM mcp_bindings", [], |r| r.get(0))
                .unwrap_or(0);
            assert_eq!(row_count, 0, "dry-run 不得向 mcp_bindings 写入数据");
        }
    }

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_three_formats_lifecycle_and_two_agent_isolation() {
    let (root, ctx) = temp_ctx("formats-isolation");

    // 配置 OpenCode agent 定义
    let set_opencode = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.opencode",
                "--value",
                r#"{"display_name":"OpenCode","mcp_config":{"path":"~/.config/opencode/opencode.json","format":"opencode"}}"#,
            ],
        ),
    );
    assert!(set_opencode.ok, "{:?}", set_opencode.errors);

    // 配置多 agent MCP 服务定义
    let set_def = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.multi-server",
                "--value",
                r#"{
                    "command": "npx",
                    "args": ["-y", "@mcp/fs", "/tmp"],
                    "agents": ["claude-code", "codex", "opencode"],
                    "extra_args_by_agent": {
                        "codex": ["--codex-extra"]
                    }
                }"#,
            ],
        ),
    );
    assert!(set_def.ok, "{:?}", set_def.errors);

    let claude_path = ctx.home_dir.join(".claude.json");
    let codex_path = ctx.home_dir.join(".codex/config.toml");
    let opencode_path = ctx.home_dir.join(".config/opencode/opencode.json");

    // 1. Claude add: 只影响 Claude 配置文件，不影响 Codex 或 OpenCode
    let add_claude = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:multi-server",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(add_claude.ok, "{:?}", add_claude.errors);
    assert!(claude_path.exists(), "Claude 目标配置文件应已创建");
    assert!(!codex_path.exists(), "Codex 目标配置文件不应存在（隔离性）");
    assert!(
        !opencode_path.exists(),
        "OpenCode 目标配置文件不应存在（隔离性）"
    );

    // 验证 Claude JSON 回读格式
    let claude_content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&claude_path).unwrap()).unwrap();
    let claude_server = &claude_content["mcpServers"]["multi-server"];
    assert_eq!(claude_server["command"], "npx");
    assert_eq!(claude_server["args"][0], "-y");
    assert_eq!(claude_server["env"]["_MANAGED_BY"], "agent-manager");

    // 2. Codex add: 创建 Codex TOML，不干扰 Claude
    let add_codex = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:multi-server",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(add_codex.ok, "{:?}", add_codex.errors);
    assert!(codex_path.exists(), "Codex 目标配置文件应已创建");

    // 验证 Codex TOML 回读格式（包含 extra_args 与受管注释）
    let toml_str = std::fs::read_to_string(&codex_path).unwrap();
    assert!(
        toml_str.contains("# [agent-manager:managed]"),
        "Codex TOML 必须包含受管标记注释"
    );
    assert!(
        toml_str.contains("[mcp_servers.multi-server]"),
        "Codex TOML 必须包含 server 表头"
    );
    assert!(
        toml_str.contains("--codex-extra"),
        "Codex TOML 必须包含 extra_args_by_agent 参数"
    );

    // 3. OpenCode add: 创建 OpenCode JSON，格式符合 top-level "mcp" 与 array command
    let add_opencode = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:multi-server",
                "--action",
                "add",
                "--agent",
                "opencode",
            ],
        ),
    );
    assert!(add_opencode.ok, "{:?}", add_opencode.errors);
    assert!(opencode_path.exists(), "OpenCode 目标配置文件应已创建");

    let opencode_content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&opencode_path).unwrap()).unwrap();
    let opencode_server = &opencode_content["mcp"]["multi-server"];
    assert_eq!(opencode_server["type"], "local");
    assert_eq!(opencode_server["command"][0], "npx");
    assert_eq!(
        opencode_server["environment"]["_MANAGED_BY"],
        "agent-manager"
    );

    // 4. Claude disable: 移除配置条目，但数据库绑定保留 enabled=false
    let dis_claude = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:multi-server",
                "--action",
                "disable",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(dis_claude.ok, "{:?}", dis_claude.errors);
    let claude_content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&claude_path).unwrap()).unwrap();
    assert!(
        claude_content["mcpServers"].get("multi-server").is_none(),
        "disable 必须从目标文件中移除条目"
    );

    let st = execute(&ctx, &request("mcp:status", &[]));
    assert!(st.ok);
    let el = st.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:multi-server")
        .unwrap();
    let claude_status = &el["observed"]["agents"]["claude-code"];
    assert_eq!(claude_status["assigned"], true);
    assert_eq!(claude_status["enabled"], false);
    assert_eq!(claude_status["state"], "disabled");

    // 5. Claude enable: 重新写回配置条目
    let en_claude = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:multi-server",
                "--action",
                "enable",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(en_claude.ok, "{:?}", en_claude.errors);
    let claude_content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&claude_path).unwrap()).unwrap();
    assert!(
        claude_content["mcpServers"].get("multi-server").is_some(),
        "enable 必须重新写入条目"
    );

    // 6. Claude remove: 彻底移除绑定与条目，Codex 与 OpenCode 仍然保留
    let rm_claude = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:multi-server",
                "--action",
                "remove",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(rm_claude.ok, "{:?}", rm_claude.errors);
    let claude_content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&claude_path).unwrap()).unwrap();
    assert!(claude_content["mcpServers"].get("multi-server").is_none());

    // Codex 和 OpenCode 依然完整受管
    let toml_str_after = std::fs::read_to_string(&codex_path).unwrap();
    assert!(
        toml_str_after.contains("[mcp_servers.multi-server]"),
        "Codex 条目应依然存在"
    );
    let opencode_content_after: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&opencode_path).unwrap()).unwrap();
    assert!(
        opencode_content_after["mcp"].get("multi-server").is_some(),
        "OpenCode 条目应依然存在"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_safety_unmanaged_shared_corrupt_symlinks() {
    let (root, ctx) = temp_ctx("safety-checks");

    // 写入服务定义
    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.conflict-server",
                "--value",
                r#"{"command":"node","args":["server.js"],"agents":["claude-code","codex"]}"#,
            ],
        ),
    );
    assert!(set_res.ok);

    // 1. 非受管同名条目安全冲突：目标文件中已有手工条目（无 managed 标记）
    let claude_path = ctx.home_dir.join(".claude.json");
    std::fs::create_dir_all(claude_path.parent().unwrap()).unwrap();
    std::fs::write(
        &claude_path,
        r#"{"mcpServers":{"conflict-server":{"command":"custom-node","args":["manual.js"]}}}"#,
    )
    .unwrap();

    let conflict_apply = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:conflict-server",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(!conflict_apply.ok, "存在非受管同名条目时必须拒绝覆盖");
    let err = error_messages(&conflict_apply);
    assert!(err.contains("非受管同名条目"), "应提示非受管冲突: {}", err);

    // 回读文件确认未被篡改
    let raw_claude = std::fs::read_to_string(&claude_path).unwrap();
    assert!(raw_claude.contains("custom-node"), "手工条目必须原样保留");

    // 2. 共享目标路径安全校验：两个 agent 指向完全相同的目标路径
    let set_shared = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.shared-agent",
                "--value",
                r#"{"display_name":"Shared","mcp_config":{"path":"~/.claude.json","format":"json"}}"#,
            ],
        ),
    );
    assert!(set_shared.ok);

    let shared_apply = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:conflict-server",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(!shared_apply.ok, "共享同一配置路径必须报错拦截");
    let err2 = error_messages(&shared_apply);
    assert!(err2.contains("共享 MCP 目标"), "应提示路径冲突: {}", err2);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_env_resolution_and_no_leak_guarantee() {
    let (root, ctx) = temp_ctx("env-leak-prevention");

    // 写入含有环境变量占位符的服务定义
    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.secret-server",
                "--value",
                r#"{
                    "command": "node",
                    "args": ["secret.js"],
                    "agents": ["claude-code"],
                    "env": {
                        "API_TOKEN": "${MY_SECRET_KEY_12345}"
                    }
                }"#,
            ],
        ),
    );
    assert!(set_res.ok);

    // 1. 环境变量未设置时，拒绝执行并明确告知缺失变量名
    std::env::remove_var("MY_SECRET_KEY_12345");
    let missing_env = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:secret-server",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(!missing_env.ok, "缺失环境变量应当报错");
    let err = error_messages(&missing_env);
    assert!(
        err.contains("MY_SECRET_KEY_12345"),
        "错误信息应包含缺失的变量名: {}",
        err
    );

    // 2. 设置真实敏感凭据并执行 apply
    let secret_val = "SUPER_SECRET_TOKEN_ABCD_EFGH";
    std::env::set_var("MY_SECRET_KEY_12345", secret_val);

    let ok_apply = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:secret-server",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(ok_apply.ok, "环境变量已设置时应成功: {:?}", ok_apply.errors);

    // 回读目标配置文件：包含真实解析后的值
    let claude_path = ctx.home_dir.join(".claude.json");
    let claude_str = std::fs::read_to_string(&claude_path).unwrap();
    assert!(
        claude_str.contains(secret_val),
        "目标配置文件必须写入已解析的真实环境变量"
    );

    // 3. 验证数据库中的 mcp_bindings 保留占位符，严禁泄露明文
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let def_raw: String = conn
        .query_row(
            "SELECT definition_json FROM mcp_bindings WHERE server='secret-server' AND agent='claude-code'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !def_raw.contains(secret_val),
        "mcp_bindings 数据库中严禁出现敏感明文！"
    );
    assert!(
        def_raw.contains("${MY_SECRET_KEY_12345}"),
        "数据库定义应保留 ${{VAR}} 占位符"
    );

    // 4. 验证 mcp:status 输出不泄露敏感明文
    let st = execute(&ctx, &request("mcp:status", &[]));
    assert!(st.ok);
    let st_str = serde_json::to_string(&st.data).unwrap();
    assert!(
        !st_str.contains(secret_val),
        "mcp:status 输出严禁泄露敏感明文！"
    );

    std::env::remove_var("MY_SECRET_KEY_12345");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_definition_edit_drift_and_explicit_update() {
    let (root, ctx) = temp_ctx("drift-update");

    // 初始版本 v1
    let set_v1 = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.drift-server",
                "--value",
                r#"{"command":"node","args":["v1.js"],"agents":["claude-code"]}"#,
            ],
        ),
    );
    assert!(set_v1.ok);

    let add_v1 = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:drift-server",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(add_v1.ok);

    let claude_path = ctx.home_dir.join(".claude.json");
    assert!(std::fs::read_to_string(&claude_path)
        .unwrap()
        .contains("v1.js"));

    // 修改定义为 v2
    let set_v2 = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.drift-server",
                "--value",
                r#"{"command":"node","args":["v2.js"],"agents":["claude-code"]}"#,
            ],
        ),
    );
    assert!(set_v2.ok);

    // 状态查询显示漂移 (in_sync == false)
    let st = execute(&ctx, &request("mcp:status", &[]));
    assert!(st.ok);
    let el = st.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:drift-server")
        .unwrap();
    let claude_st = &el["observed"]["agents"]["claude-code"];
    assert_eq!(
        claude_st["state"], "drifted",
        "修改定义后 binding 尚未同步，state 必须为 drifted"
    );
    assert!(claude_st["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a == "update"));

    // 此时目标文件依然是旧版 v1（不会被隐式篡改）
    assert!(std::fs::read_to_string(&claude_path)
        .unwrap()
        .contains("v1.js"));

    // 执行显式 update 动作
    let update_res = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:drift-server",
                "--action",
                "update",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(
        update_res.ok,
        "显式 update 应当成功: {:?}",
        update_res.errors
    );

    // 目标文件已更新为 v2
    assert!(std::fs::read_to_string(&claude_path)
        .unwrap()
        .contains("v2.js"));

    // 再次查询状态：恢复 in_sync == true
    let st_after = execute(&ctx, &request("mcp:status", &[]));
    let el_after = st_after.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:drift-server")
        .unwrap();
    assert_eq!(
        el_after["observed"]["agents"]["claude-code"]["state"],
        "managed-ok"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_library_remove_and_external_preservation() {
    let (root, ctx) = temp_ctx("external-preservation");

    // 准备 Codex TOML 配置文件，内含手工外部配置和注释
    let codex_path = ctx.home_dir.join(".codex/config.toml");
    std::fs::create_dir_all(codex_path.parent().unwrap()).unwrap();
    let initial_codex_toml = r#"# User custom comment
model = "gpt-4"

[mcp_servers.unmanaged_server]
command = "python"
args = ["custom.py"]
"#;
    std::fs::write(&codex_path, initial_codex_toml).unwrap();

    // 写入受管服务定义
    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.managed-one",
                "--value",
                r#"{"command":"node","args":["server.js"],"agents":["codex"]}"#,
            ],
        ),
    );
    assert!(set_res.ok);

    // 添加受管服务
    let add_res = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:managed-one",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(add_res.ok);

    // 移除受管服务
    let rm_res = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:managed-one",
                "--action",
                "remove",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(rm_res.ok);

    // 回读 Codex TOML 确认外部配置和注释完全保留
    let content = std::fs::read_to_string(&codex_path).unwrap();
    assert!(
        content.contains("# User custom comment"),
        "用户自定义注释应完整保留"
    );
    assert!(
        content.contains("[mcp_servers.unmanaged_server]"),
        "外部未受管服务必须保留"
    );
    assert!(!content.contains("managed-one"), "受管服务必须已被移除");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_locks_and_audit_logs() {
    let (root, ctx) = temp_ctx("locks-and-audit");

    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.lock-server",
                "--value",
                r#"{"command":"node","args":["server.js"],"agents":["claude-code"]}"#,
            ],
        ),
    );
    assert!(set_res.ok);

    // 1. 模拟并发持有跨进程 apply 锁
    std::fs::create_dir_all(&ctx.data_dir).unwrap();
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
                "mcp:apply",
                &[
                    "--only",
                    "mcp:lock-server",
                    "--action",
                    "add",
                    "--agent",
                    "claude",
                ],
                "contended-run",
            ),
        )
    });

    let res = worker.join().unwrap();
    assert!(!res.ok, "并发持有 apply 锁时必须立即拒绝");
    let err = error_messages(&res);
    assert!(
        err.contains("锁竞争") || err.contains("apply"),
        "应提示锁竞争: {}",
        err
    );

    // 释放锁
    drop(lock_file);

    // 2. 正常执行 mcp:apply 成功记录审计日志
    let apply_ok = execute(
        &ctx,
        &request_with_run(
            "mcp:apply",
            &[
                "--only",
                "mcp:lock-server",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
            "successful-run",
        ),
    );
    assert!(apply_ok.ok, "{:?}", apply_ok.errors);

    // 验证 runs 表中有 mcp:apply 操作记录
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let op_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM operation_logs WHERE operation_type='mcp:add' AND run_id='successful-run'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(op_count, 1, "mcp:apply 必须成功记录审计日志");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_library_actions_enable_disable_remove() {
    let (root, ctx) = temp_ctx("library-actions");

    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.lib-server",
                "--value",
                r#"{"command":"node","args":["s.js"],"agents":["claude-code"]}"#,
            ],
        ),
    );
    assert!(set_res.ok);

    // 1. 库级 disable
    let dis_res = execute(
        &ctx,
        &request(
            "mcp:apply",
            &["--only", "mcp:lib-server", "--action", "disable"],
        ),
    );
    assert!(dis_res.ok, "库级 disable 应当成功: {:?}", dis_res.errors);

    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = st.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:lib-server")
        .unwrap();
    assert_eq!(el["policy"]["enabled"], false);
    assert_eq!(el["state"], "disabled");

    // 2. 库级 enable
    let en_res = execute(
        &ctx,
        &request(
            "mcp:apply",
            &["--only", "mcp:lib-server", "--action", "enable"],
        ),
    );
    assert!(en_res.ok, "库级 enable 应当成功: {:?}", en_res.errors);

    let st_en = execute(&ctx, &request("mcp:status", &[]));
    let el_en = st_en.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:lib-server")
        .unwrap();
    assert_eq!(el_en["policy"]["enabled"], true);

    // 3. 库级 remove
    let rm_res = execute(
        &ctx,
        &request(
            "mcp:apply",
            &["--only", "mcp:lib-server", "--action", "remove"],
        ),
    );
    assert!(rm_res.ok, "库级 remove 应当成功: {:?}", rm_res.errors);

    let st_rm = execute(&ctx, &request("mcp:status", &[]));
    let el_rm = st_rm.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:lib-server");
    assert!(
        el_rm.is_none(),
        "库级 remove 后服务定义必须在 status 中完全消失"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_symlink_rejection_target_and_ancestor() {
    let (root, ctx) = temp_ctx("symlink-rejection");

    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.sym-server",
                "--value",
                r#"{"command":"node","args":["s.js"],"agents":["claude-code","linked-agent"]}"#,
            ],
        ),
    );
    assert!(set_res.ok);

    let claude_path = ctx.home_dir.join(".claude.json");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let decoy = ctx.home_dir.join("decoy.json");
    std::fs::write(&decoy, "{}").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&decoy, &claude_path).unwrap();

    let plan_res = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:sym-server",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(!plan_res.ok, "目标文件为软链接时 plan 必须拒绝");
    assert!(error_messages(&plan_res).contains("符号链接"));

    let apply_res = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:sym-server",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(!apply_res.ok, "目标文件为软链接时 apply 必须拒绝");
    assert!(error_messages(&apply_res).contains("符号链接"));
    assert_eq!(
        std::fs::read_to_string(&decoy).unwrap(),
        "{}",
        "诱饵文件不得被写入"
    );

    std::fs::remove_file(&claude_path).unwrap();

    let real_dir = ctx.home_dir.join("real_config");
    std::fs::create_dir_all(&real_dir).unwrap();
    let link_dir = ctx.home_dir.join("link_config");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();

    let set_custom_agent = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.linked-agent",
                "--value",
                &format!(
                    r#"{{"display_name":"Linked","mcp_config":{{"path":"{}/config.json","format":"claude-json"}}}}"#,
                    link_dir.to_string_lossy()
                ),
            ],
        ),
    );
    assert!(set_custom_agent.ok);

    let plan_link_res = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:sym-server",
                "--action",
                "add",
                "--agent",
                "linked-agent",
            ],
        ),
    );
    assert!(!plan_link_res.ok, "祖先包含符号链接时 plan 必须拒绝");
    assert!(error_messages(&plan_link_res).contains("符号链接"));

    let apply_link_res = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:sym-server",
                "--action",
                "add",
                "--agent",
                "linked-agent",
            ],
        ),
    );
    assert!(!apply_link_res.ok, "祖先包含符号链接时 apply 必须拒绝");
    assert!(error_messages(&apply_link_res).contains("符号链接"));
    assert!(
        !real_dir.join("config.json").exists(),
        "零写保证：真实目录中不得写入文件"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_corrupt_files_fail_closed() {
    let (root, ctx) = temp_ctx("corrupt-files");

    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.corrupt-test",
                "--value",
                r#"{"command":"node","args":["s.js"],"agents":["claude-code","codex"]}"#,
            ],
        ),
    );
    assert!(set_res.ok);

    let claude_path = ctx.home_dir.join(".claude.json");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let corrupt_json_content = "{\n  \"mcpServers\": {\n    \"broken\":\n";
    std::fs::write(&claude_path, corrupt_json_content).unwrap();

    let plan_json = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:corrupt-test",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(!plan_json.ok, "JSON 损坏时 plan 必须 fail-closed");
    let apply_json = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:corrupt-test",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(!apply_json.ok, "JSON 损坏时 apply 必须 fail-closed");
    assert_eq!(
        std::fs::read_to_string(&claude_path).unwrap(),
        corrupt_json_content,
        "JSON 损坏时不得覆盖或篡改文件"
    );

    let codex_path = ctx.home_dir.join(".codex/config.toml");
    std::fs::create_dir_all(codex_path.parent().unwrap()).unwrap();
    let corrupt_toml_content = "[mcp_servers\nunclosed table\n";
    std::fs::write(&codex_path, corrupt_toml_content).unwrap();

    let plan_toml = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:corrupt-test",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(!plan_toml.ok, "TOML 损坏时 plan 必须 fail-closed");
    let apply_toml = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:corrupt-test",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(!apply_toml.ok, "TOML 损坏时 apply 必须 fail-closed");
    assert_eq!(
        std::fs::read_to_string(&codex_path).unwrap(),
        corrupt_toml_content,
        "TOML 损坏时不得覆盖或篡改文件"
    );

    let lib_rm = execute(
        &ctx,
        &request(
            "mcp:apply",
            &["--only", "mcp:corrupt-test", "--action", "remove"],
        ),
    );
    assert!(
        !lib_rm.ok,
        "库级 remove 存在损坏目标文件时必须 fail-closed 拦截"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_unsupported_format_and_disabled_agent() {
    let (root, ctx) = temp_ctx("unsupported-and-disabled");

    let set_unsupported = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.yaml-agent",
                "--value",
                r#"{"display_name":"YamlAgent","mcp_config":{"path":"~/.yaml-agent.yml","format":"yaml"}}"#,
            ],
        ),
    );
    assert!(set_unsupported.ok);

    let set_disabled = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.disabled-agent",
                "--value",
                r#"{"display_name":"DisabledAgent","mcp_config":{"path":"~/.disabled.json","format":"claude-json"}}"#,
            ],
        ),
    );
    assert!(set_disabled.ok);

    let set_disabled_switch = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.disabled-agent",
                "--value",
                r#"{"enabled":false}"#,
            ],
        ),
    );
    assert!(set_disabled_switch.ok);

    let set_server = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.combo-server",
                "--value",
                r#"{"command":"node","args":["s.js"],"agents":["yaml-agent","disabled-agent"]}"#,
            ],
        ),
    );
    assert!(set_server.ok);

    let st = execute(&ctx, &request("mcp:status", &[]));
    assert!(st.ok);
    let el = st.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:combo-server")
        .unwrap();
    let yaml_st = &el["observed"]["agents"]["yaml-agent"];
    assert_eq!(yaml_st["state"], "unsupported");
    assert_eq!(yaml_st["actions"].as_array().unwrap().len(), 0);

    let disabled_st = &el["observed"]["agents"]["disabled-agent"];
    assert_eq!(disabled_st["state"], "disabled");
    assert_eq!(disabled_st["actions"].as_array().unwrap().len(), 0);

    let plan_unsupported = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:combo-server",
                "--action",
                "add",
                "--agent",
                "yaml-agent",
            ],
        ),
    );
    assert!(!plan_unsupported.ok);
    assert!(error_messages(&plan_unsupported).contains("不支持"));

    let apply_unsupported = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:combo-server",
                "--action",
                "add",
                "--agent",
                "yaml-agent",
            ],
        ),
    );
    assert!(!apply_unsupported.ok);
    assert!(error_messages(&apply_unsupported).contains("不支持"));

    let plan_disabled = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:combo-server",
                "--action",
                "add",
                "--agent",
                "disabled-agent",
            ],
        ),
    );
    assert!(!plan_disabled.ok);
    assert!(error_messages(&plan_disabled).contains("已被禁用"));

    let apply_disabled = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:combo-server",
                "--action",
                "add",
                "--agent",
                "disabled-agent",
            ],
        ),
    );
    assert!(!apply_disabled.ok);
    assert!(error_messages(&apply_disabled).contains("已被禁用"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_env_literal_rejection_and_placeholder_checks() {
    let (root, ctx) = temp_ctx("env-literals");

    // 1. 包含明文字面量（如 IP 或端口）的定义必须在 policy:set 阶段直接被拒绝，零写入
    let db_path = store::db_path(&ctx);
    let db_exists_before = db_path.exists();
    let db_bytes_before = if db_exists_before {
        std::fs::read(&db_path).unwrap()
    } else {
        vec![]
    };

    let set_literal = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.literal-server",
                "--value",
                r#"{
                    "command": "node",
                    "args": ["server.js"],
                    "agents": ["claude-code"],
                    "env": {
                        "HOST": "127.0.0.1"
                    }
                }"#,
            ],
        ),
    );
    assert!(
        !set_literal.ok,
        "明文字面量必须在 policy:set 阶段被直接拒绝"
    );
    assert!(error_messages(&set_literal).contains("仅允许 ${VAR} 占位符"));

    let dry_literal = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.literal-server",
                "--value",
                r#"{
                    "command": "node",
                    "args": ["server.js"],
                    "agents": ["claude-code"],
                    "env": {
                        "HOST": "127.0.0.1"
                    }
                }"#,
                "--dry-run",
            ],
        ),
    );
    assert!(
        !dry_literal.ok,
        "明文字面量在 policy:set --dry-run 也必须被直接拒绝"
    );
    assert!(error_messages(&dry_literal).contains("仅允许 ${VAR} 占位符"));

    if db_exists_before {
        assert_eq!(
            std::fs::read(&db_path).unwrap(),
            db_bytes_before,
            "数据库字节必须严格不变"
        );
    } else {
        assert!(!db_path.exists(), "失败的 policy:set 不得创建数据库文件");
    }

    let set_bad_ph = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.bad-placeholder",
                "--value",
                r#"{
                    "command": "node",
                    "args": ["server.js"],
                    "agents": ["claude-code"],
                    "env": {
                        "KEY": "$INVALID_STYLE"
                    }
                }"#,
            ],
        ),
    );
    assert!(
        !set_bad_ph.ok,
        "非 ${{VAR}} 格式占位符必须在 policy:set 时被拒绝"
    );

    let set_multi_missing = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.multi-missing",
                "--value",
                r#"{
                    "command": "node",
                    "args": ["server.js"],
                    "agents": ["claude-code"],
                    "env": {
                        "AAA_SECRET": "${MISSING_KEY_AAA}",
                        "BBB_SECRET": "${MISSING_KEY_BBB}"
                    }
                }"#,
            ],
        ),
    );
    assert!(set_multi_missing.ok);
    std::env::remove_var("MISSING_KEY_AAA");
    std::env::remove_var("MISSING_KEY_BBB");

    let apply_missing = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:multi-missing",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(!apply_missing.ok);
    let err_str = error_messages(&apply_missing);
    assert!(err_str.contains("MISSING_KEY_AAA"));
    assert!(err_str.contains("MISSING_KEY_BBB"));
    assert!(
        !err_str.contains("[\"MISSING_KEY_AAA\""),
        "严禁泄漏 debug 格式"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_unrelated_policy_set_not_blocked_by_historical_bad_mcp() {
    let (root, ctx) = temp_ctx("unrelated-policy-set");

    // 直接在数据库中写入包含明文字面量的历史坏 MCP 定义
    let conn = store::open_write(&ctx).unwrap();
    let bad_policy = serde_json::json!({
        "schema_version": "1.0",
        "mcp": {
            "definitions": {
                "historical-bad": {
                    "command": "bad",
                    "env": {
                        "SECRET": "plain_secret_value"
                    }
                }
            }
        }
    });
    store::write_policy(&conn, &bad_policy).unwrap();
    drop(conn);

    // 1. 无关 settings.theme 在 --dry-run 下成功，不应受历史坏 MCP 牵连
    let dry_settings = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "settings.theme",
                "--value",
                "\"dark\"",
                "--dry-run",
            ],
        ),
    );
    assert!(
        dry_settings.ok,
        "无关 settings.theme dry-run 不应被历史坏 MCP 阻塞"
    );

    // 2. 无关 settings.theme 实际写入成功
    let set_settings = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "settings.theme",
                "--value",
                "\"dark\"",
            ],
        ),
    );
    assert!(
        set_settings.ok,
        "无关 settings.theme 写入不应被历史坏 MCP 阻塞"
    );

    // 3. 无关 agents 策略修改成功
    let set_agent = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.claude-code.enabled",
                "--value",
                "false",
            ],
        ),
    );
    assert!(set_agent.ok, "无关 agents 策略修改不应被历史坏 MCP 阻塞");

    // 4. 设置合法的独立 MCP 定义成功
    let set_good_mcp = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.good-server",
                "--value",
                r#"{"command":"node","args":["server.js"],"env":{"PORT":"${PORT}"}}"#,
            ],
        ),
    );
    assert!(set_good_mcp.ok, "设置合法的 MCP 定义不应被历史坏 MCP 阻塞");

    // 5. 新增坏 MCP 定义必须被拒绝
    let set_another_bad = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.bad-server-2",
                "--value",
                r#"{"command":"node","args":["server.js"],"env":{"SECRET":"plain"}}"#,
            ],
        ),
    );
    assert!(
        !set_another_bad.ok,
        "新增包含明文字面量的坏 MCP 定义必须被拒绝"
    );

    // 6. 往历史坏 MCP 定义追加明文也必须被拒绝
    let set_bad_modify = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.historical-bad.env.EXTRA",
                "--value",
                "\"bad_literal\"",
            ],
        ),
    );
    assert!(!set_bad_modify.ok, "修改坏 MCP 定义注入明文必须被拒绝");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_db_corrupt_definition_fails_closed() {
    let (root, ctx) = temp_ctx("db-corrupt-def");

    let conn = store::open_write(&ctx).unwrap();
    conn.execute(
        "CREATE TABLE mcp_bindings (
            server TEXT NOT NULL,
            agent TEXT NOT NULL,
            version TEXT NOT NULL,
            definition_json TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            assigned INTEGER NOT NULL DEFAULT 1,
            applied_digest TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (server, agent)
        )",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO mcp_bindings (server, agent, version, definition_json, enabled, assigned, applied_digest)
         VALUES ('bad-server', 'claude-code', 'v1', '{corrupt json...', 1, 1, 'sha256:123')",
        [],
    ).unwrap();
    drop(conn);

    let st = execute(&ctx, &request("mcp:status", &[]));
    assert!(!st.ok, "DB 数据损坏时 status 必须 fail-closed");

    let plan_res = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:bad-server",
                "--action",
                "enable",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(!plan_res.ok, "DB 数据损坏时 plan 必须 fail-closed");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_library_switch_isolation_vs_agent_files() {
    let (root, ctx) = temp_ctx("switch-isolation");

    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.iso-server",
                "--value",
                r#"{"command":"node","args":["s.js"],"agents":["claude-code","codex"]}"#,
            ],
        ),
    );
    assert!(set_res.ok);

    let add_claude = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:iso-server",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(add_claude.ok);

    let add_codex = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:iso-server",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(add_codex.ok);

    let claude_file = ctx.home_dir.join(".claude.json");
    let codex_file = ctx.home_dir.join(".codex/config.toml");
    assert!(std::fs::read_to_string(&claude_file)
        .unwrap()
        .contains("iso-server"));
    assert!(std::fs::read_to_string(&codex_file)
        .unwrap()
        .contains("iso-server"));

    let lib_dis = execute(
        &ctx,
        &request(
            "mcp:apply",
            &["--only", "mcp:iso-server", "--action", "disable"],
        ),
    );
    assert!(lib_dis.ok);

    assert!(
        std::fs::read_to_string(&claude_file)
            .unwrap()
            .contains("iso-server"),
        "库级 disable 不得修改 Claude 目标文件"
    );
    assert!(
        std::fs::read_to_string(&codex_file)
            .unwrap()
            .contains("iso-server"),
        "库级 disable 不得修改 Codex 目标文件"
    );

    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let (claude_en, claude_as): (i64, i64) = conn
        .query_row(
            "SELECT enabled, assigned FROM mcp_bindings WHERE server='iso-server' AND agent='claude-code'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (claude_en, claude_as),
        (1, 1),
        "Agent 专属 binding 状态必须保留原样"
    );

    let plan_en = execute(
        &ctx,
        &request(
            "mcp:plan",
            &["--only", "mcp:iso-server", "--action", "enable"],
        ),
    );
    assert!(plan_en.ok);
    let acts = plan_en.data["actions"].as_array().unwrap();
    assert_eq!(acts.len(), 1);
    assert_eq!(acts[0]["op"], "write-file");

    let plan_rm = execute(
        &ctx,
        &request(
            "mcp:plan",
            &["--only", "mcp:iso-server", "--action", "remove"],
        ),
    );
    assert!(plan_rm.ok);
    let rm_acts = plan_rm.data["actions"].as_array().unwrap();
    assert!(rm_acts.iter().all(|a| a["op"] == "write-file"));
    assert!(rm_acts
        .iter()
        .any(|a| a["target"] == "native-manager.db:policy"));
    assert!(rm_acts
        .iter()
        .any(|a| a["target"] == "native-manager.db:mcp_bindings"));

    let lib_rm = execute(
        &ctx,
        &request(
            "mcp:apply",
            &["--only", "mcp:iso-server", "--action", "remove"],
        ),
    );
    assert!(lib_rm.ok);

    assert!(
        !std::fs::read_to_string(&claude_file)
            .unwrap()
            .contains("iso-server"),
        "库级 remove 必须从 Claude 文件中清理条目"
    );
    assert!(
        !std::fs::read_to_string(&codex_file)
            .unwrap()
            .contains("iso-server"),
        "库级 remove 必须从 Codex 文件中清理条目"
    );

    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM mcp_bindings WHERE server='iso-server'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "库级 remove 必须删除所有数据库 binding");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_strict_cli_and_bridge_args() {
    let (root, ctx) = temp_ctx("strict-args");

    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.arg-server",
                "--value",
                r#"{"command":"node","args":["s.js"],"agents":["claude-code"]}"#,
            ],
        ),
    );
    assert!(set_res.ok);

    let stray = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:arg-server",
                "--action",
                "add",
                "--agent",
                "claude",
                "stray_arg",
            ],
        ),
    );
    assert!(!stray.ok, "多余 token 必须拦截");
    assert!(error_messages(&stray).contains("意外的位置参数"));

    let dup = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:arg-server",
                "--action",
                "add",
                "--agent",
                "claude",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(!dup.ok, "重复 flag 必须拦截");
    assert!(error_messages(&dup).contains("重复提供参数"));

    let missing = execute(
        &ctx,
        &request(
            "mcp:apply",
            &["--only", "mcp:arg-server", "--action", "add", "--agent"],
        ),
    );
    assert!(!missing.ok, "缺失值必须拦截");
    assert!(error_messages(&missing).contains("缺少参数值"));

    let unknown = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:arg-server",
                "--action",
                "add",
                "--unknown-flag",
            ],
        ),
    );
    assert!(!unknown.ok, "未知 flag 必须拦截");
    assert!(error_messages(&unknown).contains("未知选项"));

    let bad_action = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:arg-server",
                "--action",
                "restart",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(!bad_action.ok, "非法 action 必须拦截");
    assert!(error_messages(&bad_action).contains("add/update/enable/disable/remove"));

    let bad_server_name = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:inv@lid/name",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(!bad_server_name.ok, "非法 server 名称必须拦截");
    assert!(error_messages(&bad_server_name).contains("非法 server 名称"));

    // mcp:status 接受可选的单个 --offline
    let ok_status_no_args = execute(&ctx, &request("mcp:status", &[]));
    assert!(ok_status_no_args.ok, "mcp:status 无参调用必须成功");

    let ok_status_offline = execute(&ctx, &request("mcp:status", &["--offline"]));
    assert!(ok_status_offline.ok, "mcp:status --offline 调用必须成功");

    let bad_status_dup_offline = execute(&ctx, &request("mcp:status", &["--offline", "--offline"]));
    assert!(
        !bad_status_dup_offline.ok,
        "mcp:status 重复 --offline 必须拦截"
    );
    assert!(error_messages(&bad_status_dup_offline).contains("仅接受可选的单个 --offline"));

    let bad_status_extra = execute(&ctx, &request("mcp:status", &["unexpected_extra"]));
    assert!(!bad_status_extra.ok, "mcp:status 携带其他参数必须拦截");
    assert!(error_messages(&bad_status_extra).contains("仅接受可选的单个 --offline"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_status_schema_and_plan_schema_contract() {
    let (root, ctx) = temp_ctx("schema-contract");

    let status_schema = load_contract_schema("contract/engine-v1/status.schema.json");
    let plan_schema = load_contract_schema("contract/engine-v1/plan.schema.json");
    let envelope_schema = load_contract_schema("contract/engine-v1/envelope.schema.json");

    let set_server = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.enum-server",
                "--value",
                r#"{"command":"node","args":["s.js"],"agents":["claude-code","codex"]}"#,
            ],
        ),
    );
    assert!(set_server.ok);

    // 1. 验证 mcp:status 输出真实匹配 status.schema.json 与 envelope.schema.json
    let st = execute(&ctx, &request("mcp:status", &[]));
    assert!(st.ok);

    let st_envelope = serde_json::to_value(&st).unwrap();
    validate_json_schema(&st_envelope, &envelope_schema, &envelope_schema);
    validate_json_schema(&st.data, &status_schema, &status_schema);

    // 验证 observed.agents 键名为规范 claude-code，严禁别名 claude
    let elements = st.data["elements"].as_array().unwrap();
    for el in elements {
        if let Some(agents) = el["observed"]["agents"].as_object() {
            for (k, _) in agents {
                assert_ne!(
                    k, "claude",
                    "Agent 键名严禁为别名 claude，必须为规范名 claude-code"
                );
            }
        }
    }

    // 2. 验证 mcp:plan 输出真实匹配 plan.schema.json 与 envelope.schema.json
    let plan_res = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:enum-server",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(plan_res.ok);

    let plan_envelope = serde_json::to_value(&plan_res).unwrap();
    validate_json_schema(&plan_envelope, &envelope_schema, &envelope_schema);
    validate_json_schema(&plan_res.data, &plan_schema, &plan_schema);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_external_modification_with_unknown_fields() {
    let (root, ctx) = temp_ctx("unknown-fields");

    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.mod-server",
                "--value",
                r#"{"command":"node","args":["s.js"],"agents":["claude-code"]}"#,
            ],
        ),
    );
    assert!(set_res.ok);

    let add_res = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:mod-server",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(add_res.ok);

    let claude_file = ctx.home_dir.join(".claude.json");
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&claude_file).unwrap()).unwrap();
    doc["mcpServers"]["mod-server"]["unrecognized_custom_field"] = serde_json::json!("tampered");
    let tampered_bytes = serde_json::to_string_pretty(&doc).unwrap();
    std::fs::write(&claude_file, &tampered_bytes).unwrap();

    let st = execute(&ctx, &request("mcp:status", &[]));
    assert!(st.ok);
    let el = st.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:mod-server")
        .unwrap();
    let claude_st = &el["observed"]["agents"]["claude-code"];
    assert_eq!(
        claude_st["state"], "drifted",
        "注入未知字段后必须检测为漂移"
    );

    // 关键判别断言：执行 update / apply 必须被拒绝且目标文件字节不变
    let update_res = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:mod-server",
                "--action",
                "update",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(!update_res.ok, "包含未受管外部属性时 apply update 必须拒绝");
    assert!(error_messages(&update_res).contains("未受管外部属性"));
    assert_eq!(
        std::fs::read_to_string(&claude_file).unwrap(),
        tampered_bytes,
        "拒绝后文件字节必须严格不变"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_apply_registers_successful_writes_for_atomic_rollback() {
    let (root, ctx) = temp_ctx("atomic-rollback");

    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.rb-server",
                "--value",
                r#"{"command":"node","args":["s.js"],"agents":["claude-code","codex"]}"#,
            ],
        ),
    );
    assert!(set_res.ok);

    let claude_file = ctx.home_dir.join(".claude.json");
    let codex_file = ctx.home_dir.join(".codex/config.toml");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    std::fs::create_dir_all(codex_file.parent().unwrap()).unwrap();

    let add_c = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:rb-server",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        ),
    );
    assert!(add_c.ok);

    let add_x = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:rb-server",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(add_x.ok);

    let claude_before_rm = std::fs::read_to_string(&claude_file).unwrap();
    let codex_before_rm = std::fs::read_to_string(&codex_file).unwrap();

    // 将 codex_file 所在目录设为只读，导致 preflight（只读）通过，
    // 而 apply 时首目标 Claude 写入成功，第二目标 Codex 在 atomic_write 创建临时文件时失败！
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let parent = codex_file.parent().unwrap();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o555)).unwrap();
    }

    let policy_before = store::effective_policy(&ctx).unwrap();

    let rm_fail = execute(
        &ctx,
        &request(
            "mcp:apply",
            &["--only", "mcp:rb-server", "--action", "remove"],
        ),
    );
    assert!(!rm_fail.ok, "第二目标写入失败时操作必须失败");

    let policy_after = store::effective_policy(&ctx).unwrap();
    assert_eq!(
        policy_before, policy_after,
        "多目标回滚后策略候选前后必须严格一致"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let parent = codex_file.parent().unwrap();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // 回读验证：首目标 Claude 文件在回滚后完全恢复原字节！
    let claude_after = std::fs::read_to_string(&claude_file).unwrap();
    assert_eq!(
        claude_after, claude_before_rm,
        "首目标已写文件必须经回滚完全恢复"
    );
    let codex_after = std::fs::read_to_string(&codex_file).unwrap();
    assert_eq!(codex_after, codex_before_rm, "第二目标文件亦必须保持不变");

    // 验证 DB 中的 binding 仍然完整存在（事务未提交）
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM mcp_bindings WHERE server='rb-server'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2, "事务回滚必须保证数据库两项 binding 均未删除");

    // 验证 operation_logs 未记录该失败操作
    let op_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM operation_logs WHERE operation_type='mcp:remove' AND run_id='mcp-test-run'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(op_count, 0, "事务回滚必须保证未写入 operation_logs");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_safe_atomic_io_modes_and_cleanup() {
    let (root, ctx) = temp_ctx("safe-atomic-io");

    // 1. 测试创建新文件时 mode 权限为 0o600 (Unix)
    let set_def = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.s-io",
                "--value",
                r#"{"command": "echo", "args": ["hi"], "agents": ["claude-code"]}"#,
            ],
        ),
    );
    assert!(set_def.ok);

    let apply_res = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:s-io",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(apply_res.ok);

    let claude_cfg = ctx.home_dir.join(".claude.json");
    assert!(claude_cfg.exists());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::symlink_metadata(&claude_cfg).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "新创建的目标配置文件必须具备 0o600 权限");
    }

    // 2. 测试写入失败时清理临时文件，不留残余
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let parent = claude_cfg.parent().unwrap();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o555)).unwrap();
        let fail_res = execute(
            &ctx,
            &request(
                "mcp:apply",
                &[
                    "--only",
                    "mcp:s-io",
                    "--action",
                    "remove",
                    "--agent",
                    "claude-code",
                ],
            ),
        );
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!fail_res.ok, "向只读目录执行 apply 必须失败");
    }

    // 检查 claude 所在目录下不存在以 .tmp_ 为后缀的临时文件
    let parent_dir = claude_cfg.parent().unwrap();
    let tmp_files: Vec<_> = std::fs::read_dir(parent_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp_"))
        .collect();
    assert!(tmp_files.is_empty(), "原子写入失败时不应残留任何临时文件");

    // 3. 测试不可读文件 (mode 0000) 导致 read_existing_bytes / guard.record fail-closed，且不破坏文件
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&claude_cfg, std::fs::Permissions::from_mode(0o000)).unwrap();

        let unreadable_res = execute(
            &ctx,
            &request(
                "mcp:apply",
                &[
                    "--only",
                    "mcp:s-io",
                    "--action",
                    "remove",
                    "--agent",
                    "claude-code",
                ],
            ),
        );
        assert!(
            !unreadable_res.ok,
            "目标文件无法读取时必须 fail-closed 报错"
        );

        // 恢复权限并验证内容完好
        std::fs::set_permissions(&claude_cfg, std::fs::Permissions::from_mode(0o600)).unwrap();
        let content = std::fs::read_to_string(&claude_cfg).unwrap();
        assert!(content.contains("s-io"), "无法读取时不应被覆盖或清空");
    }

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_invalid_definition_env_fail_closed() {
    let (root, ctx) = temp_ctx("invalid-env-fail-closed");

    // 1. env 必须是 object，非 object（如 string 或 array）必须在 policy:set 阶段被直接拒绝
    let db_path = store::db_path(&ctx);
    let db_bytes_before = if db_path.exists() {
        std::fs::read(&db_path).unwrap()
    } else {
        vec![]
    };

    let set_str_env = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.str-env-server",
                "--value",
                r#"{"command": "node", "args": [], "env": "PORT=8080"}"#,
            ],
        ),
    );
    assert!(!set_str_env.ok, "env 为 string 必须被拒绝");
    assert!(error_messages(&set_str_env).contains("必须是键值对象"));

    let set_arr_env = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.arr-env-server",
                "--value",
                r#"{"command": "node", "args": [], "env": ["PORT=8080"]}"#,
            ],
        ),
    );
    assert!(!set_arr_env.ok, "env 为 array 必须被拒绝");
    assert!(error_messages(&set_arr_env).contains("必须是键值对象"));

    // 证明 DB 零写入
    let db_bytes_after = if db_path.exists() {
        std::fs::read(&db_path).unwrap()
    } else {
        vec![]
    };
    assert_eq!(
        db_bytes_before, db_bytes_after,
        "校验失败时数据库必须零写入"
    );

    // 2. 历史遗留明文字面量在 status 中必须脱敏为 <literal>，零 secret 泄露
    let conn = store::open_write(&ctx).unwrap();
    let mut policy = store::builtin_policy();
    policy["mcp"]["definitions"]["legacy-secret-server"] = serde_json::json!({
        "command": "node",
        "args": [],
        "env": {
            "SECRET_TOKEN": "super-secret-password-123",
            "SAFE_VAR": "${SAFE_VAR}"
        }
    });
    store::write_policy(&conn, &policy).unwrap();
    drop(conn);

    let status = execute(&ctx, &request("mcp:status", &[]));
    assert!(status.ok);
    let status_json = serde_json::to_string(&status.data).unwrap();
    assert!(
        !status_json.contains("super-secret-password-123"),
        "mcp:status 全文绝不能泄漏历史字面量密码"
    );

    let server_entry = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "mcp:legacy-secret-server")
        .expect("应发现 legacy-secret-server");
    assert_eq!(
        server_entry["spec"]["env"]["SECRET_TOKEN"], "<literal>",
        "字面量必须被脱敏为 <literal>"
    );
    assert_eq!(
        server_entry["spec"]["env"]["SAFE_VAR"], "${SAFE_VAR}",
        "合法的占位符应保持原样"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_scope_and_canonical_identifiers() {
    let (root, ctx) = temp_ctx("scope-identifiers");

    // 1. allowed_agents 非法类型（如 "agents": "all" 或 [123]）fail-closed
    let set_bad_agents = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.bad-agents",
                "--value",
                r#"{"command": "echo", "args": [], "agents": "all"}"#,
            ],
        ),
    );
    assert!(!set_bad_agents.ok, "agents 为非数组必须在 policy:set 拒绝");

    let set_bad_elements = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.bad-elements",
                "--value",
                r#"{"command": "echo", "args": [], "agents": [123]}"#,
            ],
        ),
    );
    assert!(
        !set_bad_elements.ok,
        "agents 元素为非字符串必须在 policy:set 拒绝"
    );

    // 2. 测试非规范 agent 标识（如 "claude" 而不是 "claude-code"）被拒绝
    let conn = store::open_write(&ctx).unwrap();
    conn.execute_batch(agent_manager_engine::native_mcp::model::TABLE_DDL)
        .unwrap();
    let ins_bad_agent = conn.execute(
        "INSERT INTO mcp_bindings (server, agent, version, definition_json, enabled, assigned, applied_digest, updated_at)
         VALUES ('srv1', 'claude', 'v1', '{}', 1, 1, 'sha256:123', '2026-01-01T00:00:00Z')",
        [],
    );
    assert!(ins_bad_agent.is_ok());

    // 当读取 bindings 时，validate_binding 拒绝非规范 agent
    let status_bad = execute(&ctx, &request("mcp:status", &[]));
    assert!(
        !status_bad.ok,
        "非规范 agent 标识必须导致 status fail-closed 报错"
    );
    assert!(error_messages(&status_bad).contains("非规范"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_action_plan_consistency_and_leftovers() {
    let (root, ctx) = temp_ctx("plan-leftovers");

    // 1. 定义合法的 server
    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.srv-leftover",
                "--value",
                r#"{"command": "echo", "args": ["lo"], "agents": ["claude-code"]}"#,
            ],
        ),
    );
    assert!(set_res.ok);

    // 构造 managed leftover：配置文件中有 entry 且标记为 managed，但 DB 中没有该 binding
    let claude_cfg = ctx.home_dir.join(".claude.json");
    std::fs::create_dir_all(claude_cfg.parent().unwrap()).unwrap();
    std::fs::write(
        &claude_cfg,
        r#"{
            "mcpServers": {
                "srv-leftover": {
                    "command": "echo",
                    "args": ["lo"],
                    "env": {
                        "_MANAGED_BY": "agent-manager"
                    }
                }
            }
        }"#,
    )
    .unwrap();

    // 2. 验证 preflight: managed leftover 必须拒绝 add 和 enable
    let plan_add = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-leftover",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(!plan_add.ok, "存在托管残留时必须拒绝 add");
    assert!(error_messages(&plan_add).contains("残留受管条目"));

    let plan_enable = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-leftover",
                "--action",
                "enable",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(!plan_enable.ok, "存在托管残留时必须拒绝 enable");
    assert!(error_messages(&plan_enable).contains("尚未在 Agent"));

    // 3. 未受管外部字段在缺少环境变量解析时仍能以结构性检查拒绝，无需先解析 secret
    let set_missing_env = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.srv-struct-check",
                "--value",
                r#"{
                    "command": "node",
                    "args": ["server.js"],
                    "agents": ["claude-code"],
                    "env": {"TOKEN": "${DEFINITELY_UNSET_VAR_XYZ}"}
                }"#,
            ],
        ),
    );
    assert!(set_missing_env.ok);

    // 写入含有未知外部属性的配置文件条目（同时保留 srv-leftover）
    let claude_cfg = ctx.home_dir.join(".claude.json");
    std::fs::write(
        &claude_cfg,
        r#"{
            "mcpServers": {
                "srv-leftover": {
                    "command": "echo",
                    "args": ["lo"],
                    "env": {
                        "_MANAGED_BY": "agent-manager"
                    }
                },
                "srv-struct-check": {
                    "command": "node",
                    "args": ["server.js"],
                    "unknown_external_attr": "custom_val",
                    "env": {
                        "_MANAGED_BY": "agent-manager"
                    }
                }
            }
        }"#,
    )
    .unwrap();

    let plan_struct = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-struct-check",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(!plan_struct.ok);
    assert!(
        error_messages(&plan_struct).contains("未受管外部属性")
            || error_messages(&plan_struct).contains("外部未识别字段"),
        "结构性未知字段检查必须在环境变量解析之前拒绝: {}",
        error_messages(&plan_struct)
    );

    // 检查 mcp:status：包含未受管外部属性时单元格不得暴露可写动作
    let st_struct = execute(&ctx, &request("mcp:status", &[]));
    assert!(st_struct.ok);
    let struct_elem = st_struct.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:srv-struct-check")
        .expect("srv-struct-check 必须在 status 中显示");
    let struct_cell = &struct_elem["observed"]["agents"]["claude-code"];
    assert_eq!(struct_cell["state"], "drifted");
    assert!(
        struct_cell["actions"].as_array().unwrap().is_empty(),
        "包含未受管外部属性时单元格不得暴露可写动作"
    );

    // 4. 验证残留受管条目 (leftover) 在 status 中的呈现及动作可执行性
    let srv_lo_elem = st_struct.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:srv-leftover")
        .expect("srv-leftover 必须在 status 中显示");
    let lo_cell = &srv_lo_elem["observed"]["agents"]["claude-code"];
    assert_eq!(lo_cell["state"], "leftover");
    assert_eq!(lo_cell["actions"], serde_json::json!(["remove"]));
    let plan_lo_rm = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-leftover",
                "--action",
                "remove",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(
        plan_lo_rm.ok,
        "leftover 状态暴露的 remove 动作必须可通过 mcp:plan 执行: {:?}",
        plan_lo_rm.errors
    );

    // 5. 绑定已停用 (enabled: false) 但配置文件中仍有残留受管条目
    let conn = store::open_write(&ctx).unwrap();
    agent_manager_engine::native_mcp::persistence::ensure_table(&conn).unwrap();
    let lo_def = serde_json::json!({"command": "echo", "args": ["lo"], "agents": ["claude-code"]});
    let lo_version = agent_manager_engine::native_mcp::compute_definition_version(&lo_def);
    conn.execute(
        "INSERT INTO mcp_bindings (server, agent, version, definition_json, enabled, assigned, applied_digest, updated_at)
         VALUES ('srv-leftover', 'claude-code', ?1, ?2, 0, 1, '', '2026-09-11T00:00:00Z')",
        rusqlite::params![lo_version, serde_json::to_string(&lo_def).unwrap()],
    ).unwrap();
    drop(conn);

    let st_dis_res = execute(&ctx, &request("mcp:status", &[]));
    assert!(st_dis_res.ok, "st_dis_res errors: {:?}", st_dis_res.errors);
    let dis_res_elem = st_dis_res.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:srv-leftover")
        .unwrap();
    let dis_res_cell = &dis_res_elem["observed"]["agents"]["claude-code"];
    assert_eq!(dis_res_cell["state"], "drifted");
    let dis_res_acts: Vec<String> = dis_res_cell["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        dis_res_acts.contains(&"disable".to_string())
            && dis_res_acts.contains(&"remove".to_string())
    );
    assert!(
        !dis_res_acts.contains(&"add".to_string())
            && !dis_res_acts.contains(&"enable".to_string())
            && !dis_res_acts.contains(&"update".to_string())
    );

    // 验证暴露的 disable 和 remove 均可通过 mcp:plan 执行
    let plan_dis_clean = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-leftover",
                "--action",
                "disable",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(
        plan_dis_clean.ok,
        "已停用但有残留时暴露的 disable 必须可通过 plan 执行: {:?}",
        plan_dis_clean.errors
    );

    let plan_rm_clean = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-leftover",
                "--action",
                "remove",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(
        plan_rm_clean.ok,
        "已停用但有残留时暴露的 remove 必须可通过 plan 执行: {:?}",
        plan_rm_clean.errors
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_plan_after_digest_matches_applied() {
    use sha2::{Digest, Sha256};
    let (root, ctx) = temp_ctx("plan-digest-matches");

    // 1. 设置包含两个 server 的策略
    execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.srv-a",
                "--value",
                r#"{"command": "echo", "args": ["a"], "agents": ["claude-code"]}"#,
            ],
        ),
    );
    execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.srv-b",
                "--value",
                r#"{"command": "echo", "args": ["b"], "agents": ["claude-code"]}"#,
            ],
        ),
    );

    // 2. 先规划并应用 srv-a
    let plan_a = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-a",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(plan_a.ok);
    let plan_a_digest = plan_a.data["actions"][0]["after_digest"]
        .as_str()
        .unwrap()
        .to_string();

    let apply_a = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:srv-a",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(apply_a.ok);

    let claude_cfg = ctx.home_dir.join(".claude.json");
    let actual_bytes_a = std::fs::read(&claude_cfg).unwrap();
    let actual_sha_a = format!("sha256:{:x}", Sha256::digest(&actual_bytes_a));
    assert_eq!(
        plan_a_digest, actual_sha_a,
        "add 计划中的 after_digest 必须与实际写入文件的 SHA256 严格一致"
    );

    // 3. 再规划并应用 srv-b
    let plan_b = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-b",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(plan_b.ok);
    let plan_b_digest = plan_b.data["actions"][0]["after_digest"]
        .as_str()
        .unwrap()
        .to_string();

    let apply_b = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:srv-b",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(apply_b.ok);

    let actual_bytes_b = std::fs::read(&claude_cfg).unwrap();
    let actual_sha_b = format!("sha256:{:x}", Sha256::digest(&actual_bytes_b));
    assert_eq!(
        plan_b_digest, actual_sha_b,
        "在已有 entry 上累加 entry 时 plan after_digest 亦必须与实际写入完全一致"
    );

    // 4. 规划并执行 remove srv-a（srv-b 保持保留在同一个文件中）
    let plan_rm_a = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-a",
                "--action",
                "remove",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(plan_rm_a.ok);
    let plan_rm_a_digest = plan_rm_a.data["actions"][0]["after_digest"]
        .as_str()
        .unwrap()
        .to_string();

    let apply_rm_a = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:srv-a",
                "--action",
                "remove",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(apply_rm_a.ok);

    let actual_bytes_rm = std::fs::read(&claude_cfg).unwrap();
    let actual_sha_rm = format!("sha256:{:x}", Sha256::digest(&actual_bytes_rm));
    assert_eq!(
        plan_rm_a_digest, actual_sha_rm,
        "在保留其他 entry 的情况下部分删除时，plan after_digest 必须与实际写入完全一致"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_recoverable_orphans_lifecycle() {
    let (root, ctx) = temp_ctx("orphan-lifecycle");

    // 1. 定义并应用一个跨两个 agent 的 server
    execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.orphan-srv",
                "--value",
                r#"{"command": "echo", "args": ["orphan"], "agents": ["claude-code", "codex"]}"#,
            ],
        ),
    );
    let apply_add_claude = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:orphan-srv",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(apply_add_claude.ok);
    let apply_add_codex = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:orphan-srv",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(apply_add_codex.ok);

    let claude_cfg = ctx.home_dir.join(".claude.json");
    let codex_cfg = ctx.home_dir.join(".codex").join("config.toml");
    assert!(claude_cfg.exists());
    assert!(codex_cfg.exists());

    // 2. 从策略中物理删除定义 policy:delete
    let del_pol = execute(
        &ctx,
        &request(
            "policy:delete",
            &["--layer", "local", "--key", "mcp.definitions.orphan-srv"],
        ),
    );
    assert!(del_pol.ok);

    // 3. 运行 mcp:status：orphan-srv 必须通过 bindings 被发现，状态为 drifted，action 为 ["remove"]
    let status_res = execute(&ctx, &request("mcp:status", &[]));
    assert!(status_res.ok);
    let elements = status_res.data["elements"].as_array().unwrap();
    let orphan = elements
        .iter()
        .find(|s| s["id"] == "mcp:orphan-srv")
        .expect("孤儿服务必须在 status 中显现");
    assert_eq!(orphan["state"], "drifted");
    assert_eq!(orphan["actions"][0]["id"], "remove");
    assert!(
        orphan["spec"]["command"].is_string(),
        "spec 应该展示脱敏后的历史规格"
    );

    // 4. 运行 mcp:plan remove
    let plan_rm = execute(
        &ctx,
        &request(
            "mcp:plan",
            &["--only", "mcp:orphan-srv", "--action", "remove"],
        ),
    );
    assert!(plan_rm.ok);

    // 5. 运行 mcp:apply remove，清理目标文件和 DB bindings
    let apply_rm = execute(
        &ctx,
        &request(
            "mcp:apply",
            &["--only", "mcp:orphan-srv", "--action", "remove"],
        ),
    );
    assert!(apply_rm.ok);

    // 验证目标文件中不再存在 orphan-srv
    let claude_text = std::fs::read_to_string(&claude_cfg).unwrap();
    assert!(!claude_text.contains("orphan-srv"));
    let codex_text = std::fs::read_to_string(&codex_cfg).unwrap();
    assert!(!codex_text.contains("orphan-srv"));

    // 验证 DB bindings 中不再存在 orphan-srv
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM mcp_bindings WHERE server='orphan-srv'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "apply remove 必须清理孤儿服务的全部 binding 记录");

    // 6. 再次运行 mcp:status：orphan-srv 彻底消失
    let status_after = execute(&ctx, &request("mcp:status", &[]));
    assert!(status_after.ok);
    let elements_after = status_after.data["elements"].as_array().unwrap();
    assert!(
        elements_after
            .iter()
            .find(|s| s["id"] == "mcp:orphan-srv")
            .is_none(),
        "清理后孤儿服务不应再出现在 status 中"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_library_remove_with_unsupported_agent_without_binding() {
    let (root, ctx) = temp_ctx("unsupported-remove");

    // 配置一个未支持格式的自定义 agent：在 agents.definitions.my-custom 中写入 mcp_config (yaml 格式)
    let set_custom = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.my-custom",
                "--value",
                r#"{"name":"MyCustom","enabled":true,"mcp_config":{"path":"~/.custom/cfg.yaml","format":"yaml"}}"#,
            ],
        ),
    );
    assert!(set_custom.ok);

    // 配置一个普通 server 并添加到 claude-code
    execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.srv-normal",
                "--value",
                r#"{"command":"echo","args":["hi"],"agents":["claude-code"]}"#,
            ],
        ),
    );
    let apply_add = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:srv-normal",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(apply_add.ok);

    // 检查 mcp:status：my-custom 目标单元格状态必须为 unsupported
    let st = execute(&ctx, &request("mcp:status", &[]));
    assert!(st.ok);
    let elements = st.data["elements"].as_array().unwrap();
    let srv_elem = elements
        .iter()
        .find(|e| e["id"] == "mcp:srv-normal")
        .expect("srv-normal 必须在 status 中存在");
    let custom_cell = &srv_elem["observed"]["agents"]["my-custom"];
    assert_eq!(
        custom_cell["state"], "unsupported",
        "未支持格式的目标 agent cell 必须为 unsupported: {:?}",
        custom_cell
    );

    // 库级删除 srv-normal：虽然系统中有 unsupported agent，但该 agent 没有 binding 也未在文件提及 srv-normal，库级删除必须成功！
    let plan_rm = execute(
        &ctx,
        &request(
            "mcp:plan",
            &["--only", "mcp:srv-normal", "--action", "remove"],
        ),
    );
    assert!(
        plan_rm.ok,
        "库级删除不应因未绑定的 unsupported agent 失败: {:?}",
        plan_rm.errors
    );

    let apply_rm = execute(
        &ctx,
        &request(
            "mcp:apply",
            &["--only", "mcp:srv-normal", "--action", "remove"],
        ),
    );
    assert!(apply_rm.ok, "库级应用删除必须成功: {:?}", apply_rm.errors);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_library_remove_bad_json_without_server_is_blocked() {
    let (root, ctx) = temp_ctx("remove-bad-json-hidden");
    let set_def = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.bad-json-hidden",
                "--value",
                r#"{"command":"node","agents":["claude-code"]}"#,
            ],
        ),
    );
    assert!(set_def.ok, "设置定义必须成功: {:?}", set_def.errors);
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    std::fs::write(ctx.home_dir.join(".claude.json"), "{ broken JSON").unwrap();

    let status = execute(&ctx, &request("mcp:status", &[]));
    assert!(
        status.ok,
        "坏 JSON status 必须返回结构化状态: {:?}",
        status.errors
    );
    let element = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == "mcp:bad-json-hidden")
        .expect("坏 JSON 定义必须出现在 status");
    let cell = &element["observed"]["agents"]["claude-code"];
    assert_eq!(cell["state"], "probe-error");
    assert!(cell["actions"].as_array().unwrap().is_empty());
    assert!(
        element["actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["id"] != "remove"),
        "坏 JSON element 不得暴露 remove: {:?}",
        element["actions"]
    );

    let plan = execute(
        &ctx,
        &request(
            "mcp:plan",
            &["--only", "mcp:bad-json-hidden", "--action", "remove"],
        ),
    );
    assert!(!plan.ok, "坏 JSON 即使不含 server 名也必须阻断库级 remove");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_disabled_target_nonmanaged_conflict_blocks_library_remove() {
    let (root, ctx) = temp_ctx("disabled-nonmanaged-conflict");
    let set_agent = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.disabled-conflict",
                "--value",
                r#"{"display_name":"DisabledConflict","mcp_config":{"path":"~/.disabled-conflict.json","format":"claude-json"}}"#,
            ],
        ),
    );
    assert!(
        set_agent.ok,
        "设置停用 agent 定义必须成功: {:?}",
        set_agent.errors
    );
    let set_switch = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.disabled-conflict",
                "--value",
                r#"{"enabled":false}"#,
            ],
        ),
    );
    assert!(
        set_switch.ok,
        "停用 agent 必须成功: {:?}",
        set_switch.errors
    );
    let set_server = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.disabled-conflict-server",
                "--value",
                r#"{"command":"node","agents":["disabled-conflict"]}"#,
            ],
        ),
    );
    assert!(
        set_server.ok,
        "设置服务定义必须成功: {:?}",
        set_server.errors
    );
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    std::fs::write(
        ctx.home_dir.join(".disabled-conflict.json"),
        r#"{"mcpServers":{"disabled-conflict-server":{"command":"external"}}}"#,
    )
    .unwrap();

    let status = execute(&ctx, &request("mcp:status", &[]));
    assert!(
        status.ok,
        "停用目标 status 必须返回结构化状态: {:?}",
        status.errors
    );
    let element = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == "mcp:disabled-conflict-server")
        .expect("停用目标服务必须显示");
    let cell = &element["observed"]["agents"]["disabled-conflict"];
    assert_eq!(cell["state"], "drifted");
    assert!(cell["actions"].as_array().unwrap().is_empty());
    assert!(cell["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("非受管同名条目"));
    assert!(
        element["actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["id"] != "remove"),
        "非受管冲突时 element 不得暴露 remove: {:?}",
        element["actions"]
    );

    let plan = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:disabled-conflict-server",
                "--action",
                "remove",
            ],
        ),
    );
    assert!(!plan.ok, "停用目标的非受管冲突必须阻断库级 remove");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_unknown_format_missing_target_allows_library_remove() {
    let (root, ctx) = temp_ctx("unknown-format-missing-target");
    let set_agent = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.unknown-missing",
                "--value",
                r#"{"display_name":"UnknownMissing","mcp_config":{"path":"~/.unknown-missing.cfg","format":"mystery"}}"#,
            ],
        ),
    );
    assert!(
        set_agent.ok,
        "设置 unknown agent 定义必须成功: {:?}",
        set_agent.errors
    );
    let set_server = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.unknown-format-missing",
                "--value",
                r#"{"command":"node","agents":["unknown-missing"]}"#,
            ],
        ),
    );
    assert!(
        set_server.ok,
        "设置服务定义必须成功: {:?}",
        set_server.errors
    );

    let status = execute(&ctx, &request("mcp:status", &[]));
    assert!(
        status.ok,
        "unknown format status 必须成功: {:?}",
        status.errors
    );
    let element = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == "mcp:unknown-format-missing")
        .expect("unknown format 服务必须显示");
    let cell = &element["observed"]["agents"]["unknown-missing"];
    assert_eq!(cell["state"], "unsupported");
    assert!(!cell["target_exists"].as_bool().unwrap());
    let remove_actions: Vec<&str> = element["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|action| (action["id"] == "remove").then_some(action["id"].as_str().unwrap()))
        .collect();
    assert_eq!(remove_actions, vec!["remove"]);

    for action in element["actions"].as_array().unwrap() {
        if action["id"] == "remove" {
            let plan = execute(
                &ctx,
                &request(
                    "mcp:plan",
                    &["--only", "mcp:unknown-format-missing", "--action", "remove"],
                ),
            );
            assert!(plan.ok, "暴露的 remove 必须可 plan: {:?}", plan.errors);
        }
    }
    let apply = execute(
        &ctx,
        &request(
            "mcp:apply",
            &["--only", "mcp:unknown-format-missing", "--action", "remove"],
        ),
    );
    assert!(
        apply.ok,
        "缺失目标 unknown format 的 remove 必须可 apply: {:?}",
        apply.errors
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_missing_target_file_disable_remove_zero_write() {
    let (root, ctx) = temp_ctx("missing-target-file");

    execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.srv-miss",
                "--value",
                r#"{"command":"echo","args":["miss"],"agents":["claude-code"]}"#,
            ],
        ),
    );
    let apply_add = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:srv-miss",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(apply_add.ok);

    let claude_cfg = ctx.home_dir.join(".claude.json");
    let claude_cfg_str = claude_cfg.to_str().unwrap();
    assert!(claude_cfg.exists());
    // 删除目标文件，制造文件缺失但 DB 仍有 binding 的状态
    std::fs::remove_file(&claude_cfg).unwrap();
    assert!(!claude_cfg.exists());

    // 1. plan disable: 目标文件不存在，plan 动作中绝不能包含对该文件的 write-file (op == "write-file")
    let plan_dis = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-miss",
                "--action",
                "disable",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(plan_dis.ok);
    let plan_dis_actions = plan_dis.data["actions"].as_array().unwrap();
    assert!(
        !plan_dis_actions
            .iter()
            .any(|a| a["op"] == "write-file" && a["target"] == claude_cfg_str),
        "目标文件不存在时 plan disable 绝不能生成对目标文件的 write-file 物理写入: {:?}",
        plan_dis_actions
    );
    assert!(
        plan_dis_actions.iter().any(|a| a["op"] == "write-file"
            && a["target"]
                .as_str()
                .map_or(false, |t| t.contains("native-manager.db"))),
        "plan disable 必须保留对数据库的写操作: {:?}",
        plan_dis_actions
    );

    // apply disable 应该成功更新数据库而不产生新文件
    let apply_dis = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:srv-miss",
                "--action",
                "disable",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(apply_dis.ok);
    assert!(
        !claude_cfg.exists(),
        "apply disable 不应凭空创建缺失的配置文件"
    );

    // 2. plan remove: 目标文件不存在，plan 动作中亦绝不能包含 write-file
    let plan_rm = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-miss",
                "--action",
                "remove",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(plan_rm.ok);
    let plan_rm_actions = plan_rm.data["actions"].as_array().unwrap();
    assert!(
        !plan_rm_actions
            .iter()
            .any(|a| a["op"] == "write-file" && a["target"] == claude_cfg_str),
        "目标文件不存在时 plan remove 绝不能生成对目标文件的 write-file: {:?}",
        plan_rm_actions
    );
    assert!(
        plan_rm_actions.iter().any(|a| a["op"] == "write-file"
            && a["target"]
                .as_str()
                .map_or(false, |t| t.contains("native-manager.db"))),
        "plan remove 必须保留对数据库的写操作: {:?}",
        plan_rm_actions
    );

    // apply remove 成功清理 DB binding
    let apply_rm = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:srv-miss",
                "--action",
                "remove",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(apply_rm.ok);
    assert!(!claude_cfg.exists());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_env_hash_preserves_literal_and_status_masks_literal() {
    let (root, ctx) = temp_ctx("env-hash-mask");

    // 1. 验证 compute_definition_version 保全真实 env 值的哈希（包括字面量），真实反映内容变化
    let def_lit1 =
        serde_json::json!({"command": "node", "args": ["s.js"], "env": {"API_KEY": "secret1"}});
    let def_lit2 =
        serde_json::json!({"command": "node", "args": ["s.js"], "env": {"API_KEY": "secret2"}});
    assert_ne!(
        agent_manager_engine::native_mcp::compute_definition_version(&def_lit1),
        agent_manager_engine::native_mcp::compute_definition_version(&def_lit2),
        "不同 literal env 值的定义版本 hash 必须不同（保全真实哈希）"
    );

    // 2. 验证状态中的 spec 保留标准占位符
    execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.srv-hash-1",
                "--value",
                r#"{"command":"node","args":["s.js"],"env":{"API_KEY":"${SECRET_1}"}}"#,
            ],
        ),
    );
    execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.srv-hash-2",
                "--value",
                r#"{"command":"node","args":["s.js"],"env":{"API_KEY":"${SECRET_2}"}}"#,
            ],
        ),
    );

    let st = execute(&ctx, &request("mcp:status", &[]));
    assert!(st.ok);
    let elements = st.data["elements"].as_array().unwrap();
    let srv1 = elements
        .iter()
        .find(|e| e["id"] == "mcp:srv-hash-1")
        .unwrap();
    let srv2 = elements
        .iter()
        .find(|e| e["id"] == "mcp:srv-hash-2")
        .unwrap();

    assert_eq!(srv1["spec"]["env"]["API_KEY"], "${SECRET_1}");
    assert_eq!(srv2["spec"]["env"]["API_KEY"], "${SECRET_2}");

    // 3. 模拟历史数据库绑定中存在 literal env，验证 status spec 对历史字面量脱敏为 <literal>
    let conn = store::open_write(&ctx).unwrap();
    agent_manager_engine::native_mcp::persistence::ensure_table(&conn).unwrap();
    let hist_def = serde_json::json!({
        "command": "node",
        "args": ["s.js"],
        "env": {"API_KEY": "historical-secret-value"}
    });
    let hist_version = agent_manager_engine::native_mcp::compute_definition_version(&hist_def);
    conn.execute(
        "INSERT INTO mcp_bindings (server, agent, version, definition_json, enabled, assigned, applied_digest, updated_at)
         VALUES (?1, ?2, ?3, ?4, 1, 1, '', '2026-09-11T00:00:00Z')",
        rusqlite::params!["srv-historical", "claude-code", hist_version, serde_json::to_string(&hist_def).unwrap()],
    ).unwrap();
    drop(conn);

    let st_hist = execute(&ctx, &request("mcp:status", &[]));
    assert!(
        st_hist.ok,
        "读取历史 binding 时 status 必须成功: {:?}",
        st_hist.errors
    );
    let hist_elem = st_hist.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:srv-historical")
        .expect("历史 binding 服务必须在 status 中显示");
    assert_eq!(
        hist_elem["spec"]["env"]["API_KEY"], "<literal>",
        "status spec 中的历史字面量 env 必须脱敏为 <literal>"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_fail_closed_codecs_corrupt_entries() {
    let (root, ctx) = temp_ctx("corrupt-codecs");

    // 启用 opencode 自定义 agent 定义
    execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.opencode",
                "--value",
                r#"{"display_name":"OpenCode","mcp_config":{"path":"~/.config/opencode/opencode.json","format":"opencode"}}"#,
            ],
        ),
    );

    // 定义目标 server 为 corrupt，覆盖 claude-code, codex, opencode
    execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.corrupt",
                "--value",
                r#"{"command":"node","args":["s.js"],"agents":["claude-code","codex","opencode"]}"#,
            ],
        ),
    );

    let claude_path = ctx.home_dir.join(".claude.json");
    let codex_path = ctx.home_dir.join(".codex/config.toml");
    let opencode_path = ctx.home_dir.join(".config/opencode/opencode.json");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    std::fs::create_dir_all(codex_path.parent().unwrap()).unwrap();
    std::fs::create_dir_all(opencode_path.parent().unwrap()).unwrap();

    // Helper to find mcp:corrupt element
    let find_corrupt = |st: &agent_manager_engine::Envelope| -> serde_json::Value {
        assert!(st.ok);
        st.data["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == "mcp:corrupt")
            .expect("必须按 ID 找到 mcp:corrupt 元素")
            .clone()
    };
    let assert_probe_error = |st: &agent_manager_engine::Envelope, agent: &str, reason: &str| {
        let cell = &find_corrupt(st)["observed"]["agents"][agent];
        assert_eq!(cell["state"], "probe-error", "agent={agent}");
        assert!(
            cell["actions"].as_array().unwrap().is_empty(),
            "probe-error cell 不得暴露动作: {cell:?}"
        );
        assert!(
            cell["blocked_reason"].as_str().unwrap().contains(reason),
            "probe-error 原因缺少 '{}': {:?}",
            reason,
            cell["blocked_reason"]
        );
    };

    // 1. Claude: command 非字符串 (例如数值)
    std::fs::write(
        &claude_path,
        r#"{"mcpServers":{"corrupt":{"command":123}}}"#,
    )
    .unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let cell = &find_corrupt(&st)["observed"]["agents"]["claude-code"];
    assert_eq!(cell["state"], "probe-error");
    assert!(cell["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("必须是字符串"));

    // 1b. Claude: command 只有空格、args 为 null、env 为 null 均必须拒绝
    for (content, reason) in [
        (r#"{"mcpServers":{"corrupt":{"command":"   "}}}"#, "command"),
        (
            r#"{"mcpServers":{"corrupt":{"command":"echo","args":null}}}"#,
            "args",
        ),
        (
            r#"{"mcpServers":{"corrupt":{"command":"echo","env":null}}}"#,
            "env",
        ),
    ] {
        std::fs::write(&claude_path, content).unwrap();
        let st = execute(&ctx, &request("mcp:status", &[]));
        assert_probe_error(&st, "claude-code", reason);
    }

    // 2. Claude: args 包含非字符串 (例如整数 [123])
    std::fs::write(
        &claude_path,
        r#"{"mcpServers":{"corrupt":{"command":"echo","args":[123]}}}"#,
    )
    .unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let cell = &find_corrupt(&st)["observed"]["agents"]["claude-code"];
    assert_eq!(cell["state"], "probe-error");
    assert!(cell["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("必须是字符串"));

    // 3. Claude: env 值包含非字符串 (例如数值)
    std::fs::write(
        &claude_path,
        r#"{"mcpServers":{"corrupt":{"command":"echo","env":{"PORT":8080}}}}"#,
    )
    .unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let cell = &find_corrupt(&st)["observed"]["agents"]["claude-code"];
    assert_eq!(cell["state"], "probe-error");
    assert!(cell["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("必须是字符串"));

    // 恢复 Claude 为有效内容以排除干扰
    std::fs::write(&claude_path, r#"{"mcpServers":{}}"#).unwrap();

    // 4. Codex: command 非字符串 (例如数值)
    std::fs::write(&codex_path, "[mcp_servers.corrupt]\ncommand = 123\n").unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let cell = &find_corrupt(&st)["observed"]["agents"]["codex"];
    assert_eq!(cell["state"], "probe-error");
    assert!(cell["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("必须是字符串"));

    // 5. Codex: args 包含非字符串 (例如整数 [123])
    std::fs::write(
        &codex_path,
        "[mcp_servers.corrupt]\ncommand = \"node\"\nargs = [123]\n",
    )
    .unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let cell = &find_corrupt(&st)["observed"]["agents"]["codex"];
    assert_eq!(cell["state"], "probe-error");
    assert!(cell["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("必须是字符串"));

    // 6. Codex: env 包含非字符串值
    std::fs::write(
        &codex_path,
        "[mcp_servers.corrupt]\ncommand = \"node\"\n[mcp_servers.corrupt.env]\nPORT = 8080\n",
    )
    .unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let cell = &find_corrupt(&st)["observed"]["agents"]["codex"];
    assert_eq!(cell["state"], "probe-error");
    assert!(cell["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("必须是字符串"));

    // 7. Codex: startup_timeout_sec 非法类型 (字符串 "ten")
    std::fs::write(&codex_path, "[mcp_servers.corrupt]\ncommand = \"node\"\nargs = [\"s.js\"]\nstartup_timeout_sec = \"ten\"\n").unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let cell = &find_corrupt(&st)["observed"]["agents"]["codex"];
    assert_eq!(cell["state"], "probe-error");
    assert!(cell["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("startup_timeout_sec 必须是正整数"));

    // 7b. Codex: mcp_servers 与命中 server 必须为 table，command 必须非空
    for content in [
        "mcp_servers = \"not-table\"\n",
        "[mcp_servers]\ncorrupt = \"not-table\"\n",
        "[mcp_servers.corrupt]\ncommand = \"   \"\n",
    ] {
        std::fs::write(&codex_path, content).unwrap();
        let st = execute(&ctx, &request("mcp:status", &[]));
        assert_probe_error(&st, "codex", "Codex TOML");
    }

    // 恢复 Codex 为有效内容以排除干扰
    std::fs::write(&codex_path, "").unwrap();

    // 8. OpenCode: command 不是数组或元素非字符串
    std::fs::write(&opencode_path, r#"{"mcp":{"corrupt":{"command":[123]}}}"#).unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let cell = &find_corrupt(&st)["observed"]["agents"]["opencode"];
    assert_eq!(cell["state"], "probe-error");
    assert!(cell["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("必须是字符串"));

    // 9. OpenCode: command 为空数组
    std::fs::write(&opencode_path, r#"{"mcp":{"corrupt":{"command":[]}}}"#).unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let cell = &find_corrupt(&st)["observed"]["agents"]["opencode"];
    assert_eq!(cell["state"], "probe-error");
    assert!(cell["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("command 数组不能为空"));

    // 9b. OpenCode: command 首项空白、environment 为 null、type 非 local 均拒绝
    for (content, reason) in [
        (r#"{"mcp":{"corrupt":{"command":["   "]}}}"#, "command"),
        (
            r#"{"mcp":{"corrupt":{"command":["echo"],"environment":null}}}"#,
            "environment",
        ),
        (
            r#"{"mcp":{"corrupt":{"command":["echo"],"type":null}}}"#,
            "type",
        ),
        (
            r#"{"mcp":{"corrupt":{"command":["echo"],"type":"remote"}}}"#,
            "type",
        ),
    ] {
        std::fs::write(&opencode_path, content).unwrap();
        let st = execute(&ctx, &request("mcp:status", &[]));
        assert_probe_error(&st, "opencode", reason);
    }
    let malformed_bytes = std::fs::read(&opencode_path).unwrap();
    let plan = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:corrupt",
                "--action",
                "add",
                "--agent",
                "opencode",
            ],
        ),
    );
    assert!(!plan.ok, "codec probe-error 时 plan 必须 fail-closed");
    let apply = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:corrupt",
                "--action",
                "add",
                "--agent",
                "opencode",
            ],
        ),
    );
    assert!(!apply.ok, "codec probe-error 时 apply 必须 fail-closed");
    assert_eq!(std::fs::read(&opencode_path).unwrap(), malformed_bytes);

    // 10. OpenCode: environment 包含非字符串值
    std::fs::write(
        &opencode_path,
        r#"{"mcp":{"corrupt":{"command":["echo"],"environment":{"PORT":8080}}}}"#,
    )
    .unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let cell = &find_corrupt(&st)["observed"]["agents"]["opencode"];
    assert_eq!(cell["state"], "probe-error");
    assert!(cell["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("必须是字符串"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_cleanup_remote_or_missing_command_not_blocked() {
    let (root, ctx) = temp_ctx("remote-cleanup");

    // 1. 配置一个 remote 类型的 server 定义，以及一个正常 stdio server 定义
    execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.srv-rem",
                "--value",
                r#"{"type":"remote","url":"https://example.com/mcp","agents":["claude-code"]}"#,
            ],
        ),
    );
    execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.srv-norm",
                "--value",
                r#"{"command":"node","args":["norm.js"],"agents":["claude-code"]}"#,
            ],
        ),
    );

    // 验证正常服务的 add / plan 绝不被无关的 remote 服务定义阻断
    let plan_norm = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-norm",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(
        plan_norm.ok,
        "正常服务的操作绝不能被无关的 remote 定义阻断: {:?}",
        plan_norm.errors
    );

    // 2. 库级 disable 对 remote 定义生效且不被 stdio 检查阻断
    let plan_lib_dis = execute(
        &ctx,
        &request(
            "mcp:plan",
            &["--only", "mcp:srv-rem", "--action", "disable"],
        ),
    );
    assert!(
        plan_lib_dis.ok,
        "库级 disable 动作不应被 remote 定义阻断: {:?}",
        plan_lib_dis.errors
    );

    // 3. 库级 remove 对 remote 定义生效且不被 stdio 检查阻断
    let plan_lib_rm = execute(
        &ctx,
        &request("mcp:plan", &["--only", "mcp:srv-rem", "--action", "remove"]),
    );
    assert!(
        plan_lib_rm.ok,
        "库级 remove 动作不应被 remote 定义阻断: {:?}",
        plan_lib_rm.errors
    );

    // 4. 模拟该 remote 服务已存在历史绑定，验证针对该 Agent 的 disable 和 remove 清理动作完全不依赖缺失命令
    let conn = store::open_write(&ctx).unwrap();
    agent_manager_engine::native_mcp::persistence::ensure_table(&conn).unwrap();
    let rem_def = serde_json::json!({"type":"remote"});
    let rem_version = agent_manager_engine::native_mcp::compute_definition_version(&rem_def);
    conn.execute(
        "INSERT INTO mcp_bindings (server, agent, version, definition_json, enabled, assigned, applied_digest, updated_at)
         VALUES ('srv-rem', 'claude-code', ?1, ?2, 1, 1, '', '2026-09-11T00:00:00Z')",
        rusqlite::params![rem_version, serde_json::to_string(&rem_def).unwrap()],
    ).unwrap();
    drop(conn);

    // 检查 mcp:status：暴露的动作必须仅为安全清理动作（cell: disable, remove；elem: remove），绝不暴露 add, update, enable
    let st_rem = execute(&ctx, &request("mcp:status", &[]));
    assert!(st_rem.ok);
    let srv_rem_elem = st_rem.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:srv-rem")
        .expect("srv-rem 必须在 status 中显示");
    let cell_claude = &srv_rem_elem["observed"]["agents"]["claude-code"];
    let cell_acts: Vec<String> = cell_claude["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        cell_acts.contains(&"disable".to_string()) && cell_acts.contains(&"remove".to_string()),
        "具有历史绑定的 remote 服务 cell 必须暴露安全清理动作: {:?}",
        cell_acts
    );
    assert!(
        !cell_acts.contains(&"add".to_string())
            && !cell_acts.contains(&"update".to_string())
            && !cell_acts.contains(&"enable".to_string()),
        "具有历史绑定的 remote 服务 cell 绝不能暴露 add/update/enable: {:?}",
        cell_acts
    );

    // 验证 status 暴露的所有动作均可直接通过 mcp:plan 执行成功！
    for act in srv_rem_elem["actions"].as_array().unwrap() {
        let act_id = act["id"].as_str().unwrap();
        let p = execute(
            &ctx,
            &request("mcp:plan", &["--only", "mcp:srv-rem", "--action", act_id]),
        );
        assert!(
            p.ok,
            "status 暴露的元素级动作 '{}' 必须可通过 mcp:plan 执行: {:?}",
            act_id, p.errors
        );
    }
    for (agent, cell) in srv_rem_elem["observed"]["agents"].as_object().unwrap() {
        for act in cell["actions"].as_array().unwrap() {
            let act_id = act.as_str().unwrap();
            let p = execute(
                &ctx,
                &request(
                    "mcp:plan",
                    &[
                        "--only",
                        "mcp:srv-rem",
                        "--action",
                        act_id,
                        "--agent",
                        agent,
                    ],
                ),
            );
            assert!(
                p.ok,
                "status 暴露的 agent 级动作 '{}:{}' 必须可通过 mcp:plan 执行: {:?}",
                agent, act_id, p.errors
            );
        }
    }

    // 5. 执行 disable 清理并应用
    let apply_agent_dis = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:srv-rem",
                "--action",
                "disable",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(
        apply_agent_dis.ok,
        "应用 disable 清理动作必须成功: {:?}",
        apply_agent_dis.errors
    );

    // 6. 执行 remove 清理并应用
    let apply_agent_rm = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:srv-rem",
                "--action",
                "remove",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(
        apply_agent_rm.ok,
        "应用 remove 清理动作必须成功: {:?}",
        apply_agent_rm.errors
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_invalid_definition_exposes_safe_library_remove() {
    let (root, ctx) = temp_ctx("invalid-definition-remove");
    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.remote-only",
                "--value",
                r#"{"type":"remote","url":"https://example.com/mcp"}"#,
            ],
        ),
    );
    assert!(set_res.ok, "设置无效定义必须成功: {:?}", set_res.errors);

    let status = execute(&ctx, &request("mcp:status", &[]));
    assert!(status.ok, "无效定义 status 必须可读: {:?}", status.errors);
    let element = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == "mcp:remote-only")
        .expect("无效定义必须显示为 status element");
    assert_eq!(element["state"], "unsupported");
    assert_eq!(element["actions"][0]["id"], "remove");
    assert!(element["observed"]["agents"]
        .as_object()
        .unwrap()
        .values()
        .all(|cell| cell["actions"].as_array().unwrap().is_empty()));

    let plan = execute(
        &ctx,
        &request(
            "mcp:plan",
            &["--only", "mcp:remote-only", "--action", "remove"],
        ),
    );
    assert!(plan.ok, "无效定义仍应可安全 plan remove: {:?}", plan.errors);

    let applied = execute(
        &ctx,
        &request(
            "mcp:apply",
            &["--only", "mcp:remote-only", "--action", "remove"],
        ),
    );
    assert!(
        applied.ok,
        "无效定义的库级 remove 必须可执行: {:?}",
        applied.errors
    );
    let after = execute(&ctx, &request("mcp:status", &[]));
    assert!(after.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .all(|element| element["id"] != "mcp:remote-only"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_disabled_orphan_has_no_agent_actions_but_library_remove_is_safe() {
    let (root, ctx) = temp_ctx("disabled-orphan");
    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.disabled-orphan",
                "--value",
                r#"{"command":"node","args":["orphan.js"],"agents":["claude-code"]}"#,
            ],
        ),
    );
    assert!(set_res.ok, "设置定义必须成功: {:?}", set_res.errors);
    let applied = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:disabled-orphan",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(applied.ok, "初始 add 必须成功: {:?}", applied.errors);

    assert!(
        execute(
            &ctx,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "mcp.definitions.disabled-orphan._delete",
                    "--value",
                    "true",
                ],
            ),
        )
        .ok
    );
    assert!(
        execute(
            &ctx,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "agents.claude-code.enabled",
                    "--value",
                    "false",
                ],
            ),
        )
        .ok
    );

    let status = execute(&ctx, &request("mcp:status", &[]));
    assert!(status.ok, "孤儿 status 必须可读: {:?}", status.errors);
    let element = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == "mcp:disabled-orphan")
        .expect("停用后的孤儿必须显示");
    let cell = &element["observed"]["agents"]["claude-code"];
    assert!(
        cell["actions"].as_array().unwrap().is_empty(),
        "停用 Agent cell 不得暴露逐 Agent 动作: {cell:?}"
    );
    assert_eq!(element["actions"][0]["id"], "remove");

    let agent_remove = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:disabled-orphan",
                "--action",
                "remove",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(!agent_remove.ok, "停用 Agent 的逐 Agent remove 必须被拒绝");

    let library_remove = execute(
        &ctx,
        &request(
            "mcp:plan",
            &["--only", "mcp:disabled-orphan", "--action", "remove"],
        ),
    );
    assert!(
        library_remove.ok,
        "孤儿库级 remove 必须可 plan: {:?}",
        library_remove.errors
    );
    let applied_remove = execute(
        &ctx,
        &request(
            "mcp:apply",
            &["--only", "mcp:disabled-orphan", "--action", "remove"],
        ),
    );
    assert!(
        applied_remove.ok,
        "孤儿库级 remove 必须可 apply: {:?}",
        applied_remove.errors
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_validate_definition_stdio_fail_closed() {
    use agent_manager_engine::native_mcp::policy_view::validate_definition_stdio;
    use serde_json::json;

    // 1. 合法 stdio 定义
    assert!(validate_definition_stdio(&json!({"command": "node"}), "s1").is_ok());
    assert!(
        validate_definition_stdio(&json!({"command": "node", "args": ["a", "b"]}), "s1").is_ok()
    );
    assert!(validate_definition_stdio(
        &json!({"command": "node", "extra_args_by_agent": {"claude-code": ["c"]}}),
        "s1"
    )
    .is_ok());

    // 2. command 校验：缺少、非字符串、空字符串、全空格
    assert!(validate_definition_stdio(&json!({}), "s1").is_err());
    assert!(validate_definition_stdio(&json!({"command": 123}), "s1").is_err());
    assert!(validate_definition_stdio(&json!({"command": ""}), "s1").is_err());
    assert!(validate_definition_stdio(&json!({"command": "   "}), "s1").is_err());
    assert!(validate_definition_stdio(&json!({"command": null}), "s1").is_err());

    // 3. args 校验：null 必须 fail-closed、非数组必须 fail-closed、数组内含非字符串必须 fail-closed
    assert!(validate_definition_stdio(&json!({"command": "node", "args": null}), "s1").is_err());
    assert!(
        validate_definition_stdio(&json!({"command": "node", "args": "not-array"}), "s1").is_err()
    );
    assert!(validate_definition_stdio(&json!({"command": "node", "args": [123]}), "s1").is_err());
    assert!(
        validate_definition_stdio(&json!({"command": "node", "args": ["ok", null]}), "s1").is_err()
    );
    assert!(
        validate_definition_stdio(&json!({"command": "node", "args": ["ok", true]}), "s1").is_err()
    );

    // 4. extra_args_by_agent 校验：null 必须 fail-closed、非对象必须 fail-closed、agent extra_args 为 null 必须 fail-closed
    assert!(validate_definition_stdio(
        &json!({"command": "node", "extra_args_by_agent": null}),
        "s1"
    )
    .is_err());
    assert!(validate_definition_stdio(
        &json!({"command": "node", "extra_args_by_agent": "not-obj"}),
        "s1"
    )
    .is_err());
    assert!(validate_definition_stdio(
        &json!({"command": "node", "extra_args_by_agent": {"claude-code": null}}),
        "s1"
    )
    .is_err());
    assert!(validate_definition_stdio(
        &json!({"command": "node", "extra_args_by_agent": {"claude-code": "not-array"}}),
        "s1"
    )
    .is_err());
    assert!(validate_definition_stdio(
        &json!({"command": "node", "extra_args_by_agent": {"claude-code": [123]}}),
        "s1"
    )
    .is_err());
    assert!(validate_definition_stdio(
        &json!({"command": "node", "extra_args_by_agent": {"claude-code": ["ok", null]}}),
        "s1"
    )
    .is_err());

    // 5. remote 定义在 stdio 校验下必须被拒绝
    assert!(validate_definition_stdio(&json!({"type": "remote"}), "s1").is_err());
    assert!(validate_definition_stdio(&json!({"url": "https://example.com"}), "s1").is_err());
}

#[test]
fn mcp_test_startup_timeout_json_type_is_strict_and_zero_write() {
    let (root, ctx) = temp_ctx("startup-timeout-strict");
    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.startup-strict",
                "--value",
                r#"{"command":"node","args":["server.js"],"agents":["codex"]}"#,
            ],
        ),
    );
    assert!(set_res.ok, "设置初始定义必须成功: {:?}", set_res.errors);
    let added = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:startup-strict",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(added.ok, "初始 Codex add 必须成功: {:?}", added.errors);

    let target = ctx.home_dir.join(".codex/config.toml");
    let bytes_before = std::fs::read(&target).unwrap();
    let conn_before = store::open_readonly(&ctx).unwrap().unwrap();
    let binding_before: (String, i64, i64, String, String) = conn_before
        .query_row(
            "SELECT version, enabled, assigned, applied_digest, definition_json
             FROM mcp_bindings WHERE server='startup-strict' AND agent='codex'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    drop(conn_before);

    for invalid_json in [r#""ten""#, "null", "0", "-1", "1.5"] {
        let set_invalid = execute(
            &ctx,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "mcp.definitions.startup-strict.startup_timeout_sec",
                    "--value",
                    invalid_json,
                ],
            ),
        );
        assert!(
            set_invalid.ok,
            "设置 startup_timeout_sec={} 必须成功写入策略: {:?}",
            invalid_json, set_invalid.errors
        );

        let status = execute(&ctx, &request("mcp:status", &[]));
        assert!(
            status.ok,
            "非法 timeout status 必须可读: {:?}",
            status.errors
        );
        let element = status.data["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|element| element["id"] == "mcp:startup-strict")
            .unwrap();
        let cell = &element["observed"]["agents"]["codex"];
        assert_eq!(cell["state"], "unsupported", "timeout={invalid_json}");
        let actions = cell["actions"].as_array().unwrap();
        assert!(
            actions
                .iter()
                .all(|action| !matches!(action.as_str(), Some("add" | "update" | "enable"))),
            "非法 timeout 不得暴露 add/update/enable: {actions:?}"
        );

        let plan = execute(
            &ctx,
            &request(
                "mcp:plan",
                &[
                    "--only",
                    "mcp:startup-strict",
                    "--action",
                    "update",
                    "--agent",
                    "codex",
                ],
            ),
        );
        assert!(
            !plan.ok,
            "非法 timeout 的 plan 必须 fail-closed: {invalid_json}"
        );
        let apply = execute(
            &ctx,
            &request(
                "mcp:apply",
                &[
                    "--only",
                    "mcp:startup-strict",
                    "--action",
                    "update",
                    "--agent",
                    "codex",
                ],
            ),
        );
        assert!(
            !apply.ok,
            "非法 timeout 的 apply 必须 fail-closed: {invalid_json}"
        );
        assert_eq!(std::fs::read(&target).unwrap(), bytes_before);
        let conn_after = store::open_readonly(&ctx).unwrap().unwrap();
        let binding_after: (String, i64, i64, String, String) = conn_after
            .query_row(
                "SELECT version, enabled, assigned, applied_digest, definition_json
                 FROM mcp_bindings WHERE server='startup-strict' AND agent='codex'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(binding_after, binding_before);
    }

    let positive = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.startup-strict.startup_timeout_sec",
                "--value",
                "15",
            ],
        ),
    );
    assert!(
        positive.ok,
        "正整数 timeout 必须成功设置: {:?}",
        positive.errors
    );
    let updated = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:startup-strict",
                "--action",
                "update",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(updated.ok, "正整数 timeout 必须生效: {:?}", updated.errors);
    let parsed = extract_entry(&target, Format::CodexToml, "startup-strict")
        .unwrap()
        .0
        .unwrap();
    assert_eq!(parsed.extra["startup_timeout_sec"], serde_json::json!(15));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_allowed_agents_change_with_existing_binding_safe_cleanup() {
    let (root, ctx) = temp_ctx("allowed-agents-change");

    // 1. 定义包含 claude-code 与 codex 的服务，并在 claude-code 上添加绑定
    let set_srv = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.srv-allowed",
                "--value",
                r#"{"command":"node","args":["server.js"],"agents":["claude-code","codex"]}"#,
            ],
        ),
    );
    assert!(set_srv.ok);

    let apply_add = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:srv-allowed",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(apply_add.ok, "初始 add 必须成功: {:?}", apply_add.errors);

    // 2. 修改定义：从 allowed agents 列表中移除 claude-code（仅保留 codex）
    let update_def = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.srv-allowed.agents",
                "--value",
                r#"["codex"]"#,
            ],
        ),
    );
    assert!(update_def.ok);

    // 3. 检查 status：claude-code 处于启用绑定状态但已不在允许列表中
    let st = execute(&ctx, &request("mcp:status", &[]));
    assert!(st.ok);
    let srv_elem = st.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:srv-allowed")
        .expect("srv-allowed 必须存在于 status 中");
    let cell = &srv_elem["observed"]["agents"]["claude-code"];
    assert_eq!(cell["state"], "drifted");
    assert!(
        cell["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("不在服务允许列表中"),
        "必须指出不在允许列表: {:?}",
        cell["blocked_reason"]
    );
    let cell_acts: Vec<String> = cell["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    // 绝不能暴露 preflight 会拒绝的 update / enable / add
    assert!(
        !cell_acts.contains(&"update".to_string())
            && !cell_acts.contains(&"enable".to_string())
            && !cell_acts.contains(&"add".to_string()),
        "非允许 agent 绝不能暴露 update/enable/add: {:?}",
        cell_acts
    );
    // 仍可安全暴露 disable 与 remove
    assert!(
        cell_acts.contains(&"disable".to_string()) && cell_acts.contains(&"remove".to_string()),
        "非允许 agent 的现有绑定必须暴露安全清理动作: {:?}",
        cell_acts
    );

    // 验证 preflight: 手动尝试 update 必须被明确拒绝
    let plan_update = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-allowed",
                "--action",
                "update",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(
        !plan_update.ok,
        "非允许 agent 的 update 必须被 preflight 拒绝"
    );
    assert!(error_messages(&plan_update).contains("不在服务"));

    // 验证 status 暴露的安全动作均可成功 plan
    let plan_dis = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-allowed",
                "--action",
                "disable",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(
        plan_dis.ok,
        "安全 disable 动作必须可通过 plan: {:?}",
        plan_dis.errors
    );

    let plan_rm = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-allowed",
                "--action",
                "remove",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(
        plan_rm.ok,
        "安全 remove 动作必须可通过 plan: {:?}",
        plan_rm.errors
    );

    // 4. 执行 disable 后再次检查 status：此时状态为 disabled，仅暴露 remove，绝不暴露 enable
    let apply_dis = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:srv-allowed",
                "--action",
                "disable",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(apply_dis.ok);

    let st_after_dis = execute(&ctx, &request("mcp:status", &[]));
    let cell_dis = &st_after_dis.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:srv-allowed")
        .unwrap()["observed"]["agents"]["claude-code"];
    assert_eq!(cell_dis["state"], "disabled");
    assert!(cell_dis["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("不在服务允许列表中"));
    let cell_dis_acts: Vec<String> = cell_dis["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(cell_dis_acts, vec!["remove"]);

    // 尝试 enable 必须被 preflight 拒绝
    let plan_enable = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-allowed",
                "--action",
                "enable",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(!plan_enable.ok, "非允许 agent 的 enable 必须被拒绝");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_orphan_target_probe_error_blocks_remove_until_fixed() {
    let (root, ctx) = temp_ctx("orphan-probe-error");

    // 1. 定义服务并添加到 claude-code
    execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.srv-orp-probe",
                "--value",
                r#"{"command":"node","args":["orp.js"],"agents":["claude-code"]}"#,
            ],
        ),
    );
    let apply_add = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:srv-orp-probe",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(apply_add.ok);

    // 2. 将该服务从定义中删除（标记 _delete: true），制造孤儿状态
    let del_def = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.srv-orp-probe._delete",
                "--value",
                "true",
            ],
        ),
    );
    assert!(del_def.ok);

    // 3. 破坏目标文件内容制造 probe-error
    let claude_path = ctx.home_dir.join(".claude.json");
    let valid_claude_content = std::fs::read_to_string(&claude_path).unwrap();
    std::fs::write(&claude_path, "{ broken json content !!!").unwrap();

    // 4. 检查 status：
    // - claude-code cell 必须为 probe-error，actions 为空
    // - 孤儿元素级 actions 绝不能暴露 remove（因为存在破坏的目标文件，无法安全 plan）
    let st = execute(&ctx, &request("mcp:status", &[]));
    assert!(st.ok);
    let elem = st.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:srv-orp-probe")
        .expect("孤儿服务必须在 status 中显示");
    let cell = &elem["observed"]["agents"]["claude-code"];
    assert_eq!(cell["state"], "probe-error");
    assert!(
        cell["actions"].as_array().unwrap().is_empty(),
        "probe-error cell 动作必须为空"
    );

    let elem_acts = elem["actions"].as_array().unwrap();
    assert!(
        elem_acts.is_empty(),
        "目标文件 probe-error 时孤儿元素级绝不能暴露 remove 动作: {:?}",
        elem_acts
    );

    // 验证 preflight: 此时无论是元素级 remove 还是 agent 级 remove 均必须失败拒绝
    let plan_elem_rm = execute(
        &ctx,
        &request(
            "mcp:plan",
            &["--only", "mcp:srv-orp-probe", "--action", "remove"],
        ),
    );
    assert!(
        !plan_elem_rm.ok,
        "目标损坏时孤儿 remove 必须被 preflight 拒绝"
    );

    let plan_cell_rm = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:srv-orp-probe",
                "--action",
                "remove",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(
        !plan_cell_rm.ok,
        "目标损坏时 agent 级 remove 必须被 preflight 拒绝"
    );

    // 5. 修复目标文件为有效 JSON
    std::fs::write(&claude_path, &valid_claude_content).unwrap();

    // 6. 再次检查 status：
    // - probe-error 解除，cell 暴露 remove 动作
    // - 元素级恢复暴露 remove 动作
    let st_fixed = execute(&ctx, &request("mcp:status", &[]));
    assert!(st_fixed.ok);
    let elem_fixed = st_fixed.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:srv-orp-probe")
        .unwrap();
    let cell_fixed = &elem_fixed["observed"]["agents"]["claude-code"];
    assert_ne!(cell_fixed["state"], "probe-error");
    let fixed_acts: Vec<String> = cell_fixed["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(fixed_acts.contains(&"remove".to_string()));

    let elem_fixed_acts: Vec<String> = elem_fixed["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_string())
        .collect();
    assert!(
        elem_fixed_acts.contains(&"remove".to_string()),
        "修复后必须暴露 remove 动作"
    );

    // 验证现在可以成功 plan 和 apply
    let plan_rm_ok = execute(
        &ctx,
        &request(
            "mcp:plan",
            &["--only", "mcp:srv-orp-probe", "--action", "remove"],
        ),
    );
    assert!(
        plan_rm_ok.ok,
        "修复后孤儿 remove 必须可通过 plan: {:?}",
        plan_rm_ok.errors
    );

    let apply_rm_ok = execute(
        &ctx,
        &request(
            "mcp:apply",
            &["--only", "mcp:srv-orp-probe", "--action", "remove"],
        ),
    );
    assert!(
        apply_rm_ok.ok,
        "修复后孤儿 remove 必须可通过 apply: {:?}",
        apply_rm_ok.errors
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_render_snapshot_cas_rejects_external_overwrite() {
    let (root, ctx) = temp_ctx("render-snapshot-cas");
    let path = ctx.home_dir.join(".claude.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let original = br#"{"mcpServers":{}}"#;
    std::fs::write(&path, original).unwrap();

    let rendered = render_target_content(&path, &Format::ClaudeJson, "cas-server", None).unwrap();
    assert_eq!(rendered.old_bytes.as_deref(), Some(original.as_slice()));

    let external = br#"{"mcpServers":{"external":{"command":"keep"}}}"#;
    std::fs::write(&path, external).unwrap();
    let err = atomic_write_checked(
        &path,
        &rendered.content,
        None,
        None,
        rendered.old_bytes.as_deref(),
    )
    .expect_err("渲染快照过期时必须拒绝覆盖");
    assert!(err.to_string().contains("外部修改"), "{err:#}");
    assert_eq!(std::fs::read(&path).unwrap(), external);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_rollback_cas_preserves_external_change() {
    let (root, ctx) = temp_ctx("rollback-cas");
    let path = ctx.home_dir.join(".claude.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let original = br#"original"#;
    let written = br#"written"#;
    let external = br#"external"#;
    std::fs::write(&path, original).unwrap();
    safe_write_file(&path, written).unwrap();

    let mut guard = RollbackGuard::new();
    guard.record_written(&path, Some(original.to_vec()), written.to_vec());
    std::fs::write(&path, external).unwrap();

    let err = guard
        .rollback()
        .expect_err("回滚发现外部变更时必须报告冲突");
    assert!(err.to_string().contains("回滚冲突"), "{err:#}");
    assert_eq!(std::fs::read(&path).unwrap(), external);

    let created_path = ctx.home_dir.join("created.json");
    safe_write_file(&created_path, written).unwrap();
    let mut created_guard = RollbackGuard::new();
    created_guard.record_written(&created_path, None, written.to_vec());
    std::fs::write(&created_path, external).unwrap();
    let delete_err = created_guard
        .rollback()
        .expect_err("回滚删除本次创建文件时发现外部变更必须报告冲突");
    assert!(
        delete_err.to_string().contains("回滚冲突"),
        "{delete_err:#}"
    );
    assert_eq!(std::fs::read(&created_path).unwrap(), external);

    let clean_created_path = ctx.home_dir.join("clean-created.json");
    safe_write_file(&clean_created_path, written).unwrap();
    let mut clean_created_guard = RollbackGuard::new();
    clean_created_guard.record_written(&clean_created_path, None, written.to_vec());
    clean_created_guard.rollback().unwrap();
    assert!(
        !clean_created_path.exists(),
        "未发生外部变更时应删除本次创建文件"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_direct_apply_rejects_inapplicable_state() {
    let (root, ctx) = temp_ctx("direct-apply-preflight");
    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.direct-apply",
                "--value",
                r#"{"command":"node","args":["server.js"],"agents":["claude-code"]}"#,
            ],
        ),
    );
    assert!(set_res.ok, "{set_res:?}");

    let first = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:direct-apply",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(first.ok, "{first:?}");

    let target = ctx.home_dir.join(".claude.json");
    let bytes_before = std::fs::read(&target).unwrap();
    let policy_before = store::effective_policy(&ctx).unwrap();
    let conn_before = store::open_readonly(&ctx).unwrap().unwrap();
    let binding_before: (i64, String) = conn_before
        .query_row(
            "SELECT count(*), applied_digest FROM mcp_bindings WHERE server='direct-apply' AND agent='claude-code'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    drop(conn_before);

    let err = direct_apply(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:direct-apply",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    )
    .expect_err("直接调用 apply 也必须执行适用性校验");
    assert!(err.to_string().contains("add 不适用"), "{err:#}");
    assert_eq!(std::fs::read(&target).unwrap(), bytes_before);
    assert_eq!(store::effective_policy(&ctx).unwrap(), policy_before);

    let conn_after = store::open_readonly(&ctx).unwrap().unwrap();
    let binding_after: (i64, String) = conn_after
        .query_row(
            "SELECT count(*), applied_digest FROM mcp_bindings WHERE server='direct-apply' AND agent='claude-code'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(binding_after, binding_before);

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn mcp_test_status_and_plan_reject_non_regular_targets_before_read() {
    use std::os::unix::net::UnixListener;

    let (root, ctx) = temp_ctx("non-regular-target");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let socket_path = PathBuf::from(format!("/tmp/am-mcp-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path).unwrap();
    let socket_path_str = socket_path.to_string_lossy().into_owned();
    let socket_path_json = serde_json::to_string(&socket_path_str).unwrap();

    for (name, value) in [
        (
            "mcp.definitions.socket-target",
            r#"{"command":"node","agents":["claude-code"]}"#,
        ),
        (
            "mcp.definitions.device-target",
            r#"{"command":"node","agents":["claude-code"]}"#,
        ),
    ] {
        let set_def = execute(
            &ctx,
            &request(
                "policy:set",
                &["--layer", "local", "--key", name, "--value", value],
            ),
        );
        assert!(set_def.ok, "设置测试定义必须成功: {:?}", set_def.errors);
    }
    let set_socket_path = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.claude-code.mcp_config.path",
                "--value",
                socket_path_json.as_str(),
            ],
        ),
    );
    assert!(
        set_socket_path.ok,
        "设置 socket 路径必须成功: {:?}",
        set_socket_path.errors
    );

    let status_socket = execute(&ctx, &request("mcp:status", &[]));
    assert!(
        status_socket.ok,
        "socket status 必须返回结构化状态: {:?}",
        status_socket.errors
    );
    let socket_element = status_socket.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == "mcp:socket-target")
        .unwrap();
    let socket_cell = &socket_element["observed"]["agents"]["claude-code"];
    assert_eq!(socket_cell["state"], "probe-error");
    assert!(socket_cell["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("普通文件"));
    assert!(socket_cell["actions"].as_array().unwrap().is_empty());

    let plan_socket = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:socket-target",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(!plan_socket.ok, "socket target 的 plan 必须先拒绝");

    let set_device_path = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.claude-code.mcp_config.path",
                "--value",
                r#""/dev/null""#,
            ],
        ),
    );
    assert!(
        set_device_path.ok,
        "设置设备路径必须成功: {:?}",
        set_device_path.errors
    );
    let status_device = execute(&ctx, &request("mcp:status", &[]));
    assert!(
        status_device.ok,
        "设备 status 必须返回结构化状态: {:?}",
        status_device.errors
    );
    let device_element = status_device.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == "mcp:device-target")
        .unwrap();
    let device_cell = &device_element["observed"]["agents"]["claude-code"];
    assert_eq!(device_cell["state"], "probe-error");
    assert!(device_cell["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("普通文件"));
    assert!(device_cell["actions"].as_array().unwrap().is_empty());

    let plan_device = execute(
        &ctx,
        &request(
            "mcp:plan",
            &[
                "--only",
                "mcp:device-target",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(
        !plan_device.ok,
        "/dev/null device target 的 plan 必须先拒绝"
    );

    drop(listener);
    let _ = std::fs::remove_file(&socket_path);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_direct_invalid_apply_does_not_initialize_fresh_context() {
    let (root, ctx) = temp_ctx("direct-invalid-fresh");
    let req_args = [
        "--only",
        "mcp:missing-server",
        "--action",
        "add",
        "--agent",
        "claude-code",
    ];
    assert!(!ctx.home_dir.exists());
    assert!(!ctx.data_dir.exists());

    let err = direct_apply(&ctx, &request("mcp:apply", &req_args))
        .expect_err("全新 Context 上直接无效 apply 必须被纯读 preflight 拒绝");
    assert!(err.to_string().contains("不存在") || err.to_string().contains("定义"));
    assert!(!ctx.home_dir.exists(), "无效 apply 不得创建 HOME 目录");
    assert!(!ctx.data_dir.exists(), "无效 apply 不得创建 data_dir");
    assert!(!store::db_path(&ctx).exists(), "无效 apply 不得创建 DB");

    let dry_run_args = [
        "--only",
        "mcp:missing-server",
        "--action",
        "add",
        "--agent",
        "claude-code",
        "--dry-run",
    ];
    let dry_err = direct_apply(&ctx, &request("mcp:apply", &dry_run_args))
        .expect_err("dry-run 也必须先拒绝无效状态");
    assert!(dry_err.to_string().contains("不存在") || dry_err.to_string().contains("定义"));
    assert!(!ctx.home_dir.exists(), "无效 dry-run 不得创建 HOME 目录");
    assert!(!ctx.data_dir.exists(), "无效 dry-run 不得创建 data_dir");
    assert!(!store::db_path(&ctx).exists(), "无效 dry-run 不得创建 DB");

    let _ = std::fs::remove_dir_all(root);
}
