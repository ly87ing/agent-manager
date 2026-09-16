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

/// 公开 mcp:plan 退役后的零写预检探针：与 mcp:apply 共享同一份参数解析与
/// preflight_check_state 纯读校验，不创建 lock/DB/目标文件/诊断日志。
fn preflight(ctx: &Context, args: &[&str]) -> anyhow::Result<()> {
    agent_manager_engine::native_mcp::preflight_apply(ctx, &request("mcp:apply", args))
}

/// 预检必须拒绝时返回完整错误链文本（替代旧 plan 信封上的 error_messages 断言）。
fn preflight_err(ctx: &Context, args: &[&str]) -> String {
    format!(
        "{:#}",
        preflight(ctx, args).expect_err("预检必须拒绝该动作")
    )
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

    // 2. 无定义时执行 mcp:apply 预检拦截，依然保证零写
    let err = preflight_err(
        &ctx,
        &[
            "--only",
            "mcp:nonexistent",
            "--action",
            "add",
            "--agent",
            "claude",
        ],
    );
    assert!(err.contains("不存在"), "不存在的服务定义应当拦截: {err}");
    assert!(!ctx.data_dir.exists(), "失败的预检不得创建 data_dir");

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

    // 3. 预检通过（适用性零写探针），随后真实 scoped apply 并读回验证
    preflight(
        &ctx,
        &[
            "--only",
            "mcp:test-server",
            "--action",
            "add",
            "--agent",
            "claude",
        ],
    )
    .expect("add 预检应当通过");
    assert!(
        !ctx.home_dir.join(".claude.json").exists(),
        "预检不得创建目标配置文件"
    );
    let apply_res = execute(
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
            ],
        ),
    );
    assert!(apply_res.ok, "真实 apply 应当成功: {:?}", apply_res.errors);
    assert!(
        ctx.home_dir.join(".claude.json").exists(),
        "真实 apply 必须创建目标配置文件"
    );

    // 4. mcp:apply --dry-run 已退役：必须在任何 lock/DB/目标写之前严格拒绝
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
        !dry_res.ok,
        "mcp:apply --dry-run 必须被拒绝: {:?}",
        dry_res.data
    );
    assert!(
        error_messages(&dry_res).contains("已退役 --dry-run"),
        "拒绝消息必须说明 --dry-run 已退役: {}",
        error_messages(&dry_res)
    );
    assert!(
        !dry_res.lock.held,
        "dry-run 拒绝必须在任何 lock/DB/目标写之前发生（信封 lock.held 恒为 false）"
    );

    // 检查 mcp_bindings 中不存在 dry-run 参数对应的新增行（本例中仅真实 apply 的一行）
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
            assert_eq!(row_count, 1, "dry-run 拒绝不得向 mcp_bindings 写入数据");
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

    let err = preflight_err(
        &ctx,
        &[
            "--only",
            "mcp:sym-server",
            "--action",
            "add",
            "--agent",
            "claude",
        ],
    );
    assert!(
        err.contains("符号链接"),
        "目标文件为软链接时预检必须拒绝: {err}"
    );

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

    let err_link = preflight_err(
        &ctx,
        &[
            "--only",
            "mcp:sym-server",
            "--action",
            "add",
            "--agent",
            "linked-agent",
        ],
    );
    assert!(
        err_link.contains("符号链接"),
        "祖先包含符号链接时预检必须拒绝: {err_link}"
    );

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

    assert!(
        preflight(
            &ctx,
            &[
                "--only",
                "mcp:corrupt-test",
                "--action",
                "add",
                "--agent",
                "claude",
            ],
        )
        .is_err(),
        "JSON 损坏时预检必须 fail-closed"
    );
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

    assert!(
        preflight(
            &ctx,
            &[
                "--only",
                "mcp:corrupt-test",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        )
        .is_err(),
        "TOML 损坏时预检必须 fail-closed"
    );
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

    let err_unsupported = preflight_err(
        &ctx,
        &[
            "--only",
            "mcp:combo-server",
            "--action",
            "add",
            "--agent",
            "yaml-agent",
        ],
    );
    assert!(err_unsupported.contains("不支持"));

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

    let err_disabled = preflight_err(
        &ctx,
        &[
            "--only",
            "mcp:combo-server",
            "--action",
            "add",
            "--agent",
            "disabled-agent",
        ],
    );
    assert!(err_disabled.contains("已被禁用"));

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
        "policy:set --dry-run 已退役，必须在任何写之前被拒绝"
    );
    assert!(
        error_messages(&dry_literal).contains("--dry-run"),
        "拒绝消息必须指出 --dry-run 不受支持: {}",
        error_messages(&dry_literal)
    );

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

    // 1. 无关 settings.theme 携带 --dry-run 一律作为已退役参数拒绝，历史坏 MCP 不受影响
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
        !dry_settings.ok,
        "policy:set --dry-run 已退役，必须被拒绝（与历史坏 MCP 无关）"
    );
    assert!(error_messages(&dry_settings).contains("--dry-run"));

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

    assert!(
        preflight(
            &ctx,
            &[
                "--only",
                "mcp:bad-server",
                "--action",
                "enable",
                "--agent",
                "claude",
            ],
        )
        .is_err(),
        "DB 数据损坏时预检必须 fail-closed"
    );

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

    // 库级 enable 适用性零写探针 + 真实 apply：仅翻转策略开关，Agent 文件与绑定保持原样
    preflight(&ctx, &["--only", "mcp:iso-server", "--action", "enable"])
        .expect("库级 enable 预检应当通过");

    let lib_en = execute(
        &ctx,
        &request(
            "mcp:apply",
            &["--only", "mcp:iso-server", "--action", "enable"],
        ),
    );
    assert!(lib_en.ok, "库级 enable 必须成功: {:?}", lib_en.errors);
    let st_en = execute(&ctx, &request("mcp:status", &[]));
    assert!(st_en.ok);
    let el_en = st_en.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:iso-server")
        .unwrap();
    assert_eq!(
        el_en["policy"]["enabled"], true,
        "库级 enable 必须真实翻转策略总开关"
    );
    assert!(
        std::fs::read_to_string(&claude_file)
            .unwrap()
            .contains("iso-server"),
        "库级 enable 不得修改 Claude 目标文件"
    );
    assert!(
        std::fs::read_to_string(&codex_file)
            .unwrap()
            .contains("iso-server"),
        "库级 enable 不得修改 Codex 目标文件"
    );

    // 库级 remove 适用性零写探针：随后真实 apply 清理文件、绑定与策略定义
    preflight(&ctx, &["--only", "mcp:iso-server", "--action", "remove"])
        .expect("库级 remove 预检应当通过");

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
fn mcp_test_status_schema_and_apply_envelope_contract() {
    let (root, ctx) = temp_ctx("schema-contract");

    let status_schema = load_contract_schema("contract/engine-v1/status.schema.json");

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

    // 1. 验证 mcp:status 输出真实匹配 status.schema.json；信封形状保留但不再要求 dry_run
    //    （envelope.schema.json 的 dry_run 要求随 contract 批次一并退役）
    let st = execute(&ctx, &request("mcp:status", &[]));
    assert!(st.ok);
    validate_json_schema(&st.data, &status_schema, &status_schema);
    assert!(!st.mode.offline);
    assert!(!st.lock.held, "status 永不取全局锁");
    let st_mode = serde_json::to_value(&st.mode).unwrap();
    assert!(
        st_mode.get("dry_run").is_none(),
        "mode 不得再携带 dry_run 字段: {st_mode:?}"
    );

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

    // 2. 公开 plan 已退役：真实 mcp:apply 恒取全局锁，apply 数据保留
    //    domain/steps/skipped 形状且不再携带 dry_run 字段
    let apply_res = execute(
        &ctx,
        &request(
            "mcp:apply",
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
    assert!(apply_res.ok, "{:?}", apply_res.errors);
    assert!(apply_res.lock.held, "mcp:apply 必须恒取全局锁");
    assert_eq!(apply_res.data["domain"], "mcp");
    assert!(apply_res.data["steps"].is_array());
    assert!(apply_res.data["skipped"].is_array());
    assert!(
        apply_res.data.get("dry_run").is_none(),
        "apply 结果不得再携带 dry_run 字段: {:?}",
        apply_res.data
    );

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
fn mcp_test_action_preflight_consistency_and_leftovers() {
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
    let err_add = preflight_err(
        &ctx,
        &[
            "--only",
            "mcp:srv-leftover",
            "--action",
            "add",
            "--agent",
            "claude-code",
        ],
    );
    assert!(
        err_add.contains("残留受管条目"),
        "存在托管残留时必须拒绝 add: {err_add}"
    );

    let err_enable = preflight_err(
        &ctx,
        &[
            "--only",
            "mcp:srv-leftover",
            "--action",
            "enable",
            "--agent",
            "claude-code",
        ],
    );
    assert!(
        err_enable.contains("尚未在 Agent"),
        "存在托管残留时必须拒绝 enable: {err_enable}"
    );

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

    let err_struct = preflight_err(
        &ctx,
        &[
            "--only",
            "mcp:srv-struct-check",
            "--action",
            "add",
            "--agent",
            "claude-code",
        ],
    );
    assert!(
        err_struct.contains("未受管外部属性") || err_struct.contains("外部未识别字段"),
        "结构性未知字段检查必须在环境变量解析之前拒绝: {err_struct}"
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
    preflight(
        &ctx,
        &[
            "--only",
            "mcp:srv-leftover",
            "--action",
            "remove",
            "--agent",
            "claude-code",
        ],
    )
    .expect("leftover 状态暴露的 remove 动作必须通过预检");

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

    // 验证暴露的 disable 和 remove 均可通过预检
    preflight(
        &ctx,
        &[
            "--only",
            "mcp:srv-leftover",
            "--action",
            "disable",
            "--agent",
            "claude-code",
        ],
    )
    .expect("已停用但有残留时暴露的 disable 必须通过预检");

    preflight(
        &ctx,
        &[
            "--only",
            "mcp:srv-leftover",
            "--action",
            "remove",
            "--agent",
            "claude-code",
        ],
    )
    .expect("已停用但有残留时暴露的 remove 必须通过预检");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_render_digest_matches_applied() {
    use agent_manager_engine::native_mcp::policy_view::build_entry;
    use sha2::{Digest, Sha256};
    let (root, ctx) = temp_ctx("render-digest-matches");

    // 1. 设置包含两个 server 的策略
    let def_a = serde_json::json!({"command": "echo", "args": ["a"], "agents": ["claude-code"]});
    let def_b = serde_json::json!({"command": "echo", "args": ["b"], "agents": ["claude-code"]});
    for (name, def) in [("srv-a", &def_a), ("srv-b", &def_b)] {
        let set_res = execute(
            &ctx,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    &format!("mcp.definitions.{name}"),
                    "--value",
                    &serde_json::to_string(def).unwrap(),
                ],
            ),
        );
        assert!(set_res.ok, "{:?}", set_res.errors);
    }

    let claude_cfg = ctx.home_dir.join(".claude.json");
    let applied_digest_of = |ctx: &Context, server: &str| -> String {
        store::open_readonly(ctx)
            .unwrap()
            .unwrap()
            .query_row(
                "SELECT applied_digest FROM mcp_bindings WHERE server=?1 AND agent='claude-code'",
                rusqlite::params![server],
                |r| r.get(0),
            )
            .unwrap()
    };

    // 2. srv-a：渲染摘要（apply 共享同一渲染器）+ 真实 apply 后读回，两者必须严格一致
    let entry_a = build_entry(&def_a, "srv-a", Format::ClaudeJson, "claude-code").unwrap();
    let rendered_a =
        render_target_content(&claude_cfg, &Format::ClaudeJson, "srv-a", Some(&entry_a)).unwrap();
    let expected_a = format!("sha256:{:x}", Sha256::digest(rendered_a.content.as_bytes()));

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

    let actual_sha_a = format!(
        "sha256:{:x}",
        Sha256::digest(std::fs::read(&claude_cfg).unwrap())
    );
    assert_eq!(
        expected_a, actual_sha_a,
        "add 渲染摘要必须与实际写入文件的 SHA256 严格一致"
    );
    assert_eq!(
        applied_digest_of(&ctx, "srv-a"),
        entry_a.digest(),
        "绑定读回的 applied_digest 必须等于渲染条目摘要"
    );

    // 3. srv-b：在已有 entry 上累加时渲染摘要同样必须与落地字节一致
    let entry_b = build_entry(&def_b, "srv-b", Format::ClaudeJson, "claude-code").unwrap();
    let rendered_b =
        render_target_content(&claude_cfg, &Format::ClaudeJson, "srv-b", Some(&entry_b)).unwrap();
    let expected_b = format!("sha256:{:x}", Sha256::digest(rendered_b.content.as_bytes()));

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

    let actual_sha_b = format!(
        "sha256:{:x}",
        Sha256::digest(std::fs::read(&claude_cfg).unwrap())
    );
    assert_eq!(
        expected_b, actual_sha_b,
        "在已有 entry 上累加 entry 时渲染摘要亦必须与实际写入完全一致"
    );
    assert_eq!(applied_digest_of(&ctx, "srv-b"), entry_b.digest());

    // 4. remove srv-a（srv-b 保留在同一个文件中）：渲染删除结果必须与落地字节一致
    let rendered_rm = render_target_content(&claude_cfg, &Format::ClaudeJson, "srv-a", None)
        .expect("删除渲染必须成功");
    let expected_rm = format!(
        "sha256:{:x}",
        Sha256::digest(rendered_rm.content.as_bytes())
    );

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

    let actual_sha_rm = format!(
        "sha256:{:x}",
        Sha256::digest(std::fs::read(&claude_cfg).unwrap())
    );
    assert_eq!(
        expected_rm, actual_sha_rm,
        "在保留其他 entry 的情况下部分删除时，渲染摘要必须与实际写入完全一致"
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

    // 4. 库级 remove 适用性零写探针（旧公开 plan 的等价预检）
    preflight(&ctx, &["--only", "mcp:orphan-srv", "--action", "remove"])
        .expect("孤儿服务库级 remove 预检必须通过");

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
    preflight(&ctx, &["--only", "mcp:srv-normal", "--action", "remove"])
        .expect("库级删除不应因未绑定的 unsupported agent 失败");

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

    assert!(
        preflight(
            &ctx,
            &["--only", "mcp:bad-json-hidden", "--action", "remove"]
        )
        .is_err(),
        "坏 JSON 即使不含 server 名也必须阻断库级 remove"
    );

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

    assert!(
        preflight(
            &ctx,
            &[
                "--only",
                "mcp:disabled-conflict-server",
                "--action",
                "remove"
            ],
        )
        .is_err(),
        "停用目标的非受管冲突必须阻断库级 remove"
    );

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
            preflight(
                &ctx,
                &["--only", "mcp:unknown-format-missing", "--action", "remove"],
            )
            .expect("暴露的 remove 必须通过预检");
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
    assert!(claude_cfg.exists());
    // 删除目标文件，制造文件缺失但 DB 仍有 binding 的状态
    std::fs::remove_file(&claude_cfg).unwrap();
    assert!(!claude_cfg.exists());

    // 1. 目标文件不存在时 disable 必须可适用（零写探针），真实 apply 只写数据库、不创建目标文件
    preflight(
        &ctx,
        &[
            "--only",
            "mcp:srv-miss",
            "--action",
            "disable",
            "--agent",
            "claude-code",
        ],
    )
    .expect("目标文件缺失时 disable 预检必须通过");

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
    {
        let conn = store::open_readonly(&ctx).unwrap().unwrap();
        let (enabled, assigned): (i64, i64) = conn
            .query_row(
                "SELECT enabled, assigned FROM mcp_bindings WHERE server='srv-miss' AND agent='claude-code'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (enabled, assigned),
            (0, 1),
            "apply disable 必须真实落库：enabled=0 且保留分配"
        );
    }

    // 2. 目标文件不存在时 remove 必须可适用（零写探针），真实 apply 只清理 DB binding
    preflight(
        &ctx,
        &[
            "--only",
            "mcp:srv-miss",
            "--action",
            "remove",
            "--agent",
            "claude-code",
        ],
    )
    .expect("目标文件缺失时 remove 预检必须通过");

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
    {
        let conn = store::open_readonly(&ctx).unwrap().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM mcp_bindings WHERE server='srv-miss'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "apply remove 必须真实清理数据库 binding");
    }

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
    assert!(
        preflight(
            &ctx,
            &[
                "--only",
                "mcp:corrupt",
                "--action",
                "add",
                "--agent",
                "opencode",
            ],
        )
        .is_err(),
        "codec probe-error 时预检必须 fail-closed"
    );
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

    // 1. 直写库模拟历史遗留的非法 remote 定义（缺 url；保存期校验上线前的存量数据），
    //    以及一个正常 stdio server 定义
    let mut policy = store::builtin_policy();
    policy["mcp"]["definitions"] = serde_json::json!({
        "srv-rem": {"type": "remote", "agents": ["claude-code"]}
    });
    let conn = store::open_write(&ctx).unwrap();
    store::write_policy(&conn, &policy).unwrap();
    drop(conn);
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

    // 验证正常服务的 add 预检绝不被无关的 remote 服务定义阻断
    preflight(
        &ctx,
        &[
            "--only",
            "mcp:srv-norm",
            "--action",
            "add",
            "--agent",
            "claude-code",
        ],
    )
    .expect("正常服务的操作绝不能被无关的 remote 定义阻断");

    // 2. 库级 disable 对 remote 定义生效且不被 stdio 检查阻断
    preflight(&ctx, &["--only", "mcp:srv-rem", "--action", "disable"])
        .expect("库级 disable 动作不应被 remote 定义阻断");

    // 3. 库级 remove 对 remote 定义生效且不被 stdio 检查阻断
    preflight(&ctx, &["--only", "mcp:srv-rem", "--action", "remove"])
        .expect("库级 remove 动作不应被 remote 定义阻断");

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

    // 验证 status 暴露的所有动作均可直接通过预检（零写适用性校验）！
    for act in srv_rem_elem["actions"].as_array().unwrap() {
        let act_id = act["id"].as_str().unwrap();
        preflight(&ctx, &["--only", "mcp:srv-rem", "--action", act_id])
            .unwrap_or_else(|e| panic!("status 暴露的元素级动作 '{act_id}' 必须通过预检: {e:#}"));
    }
    for (agent, cell) in srv_rem_elem["observed"]["agents"].as_object().unwrap() {
        for act in cell["actions"].as_array().unwrap() {
            let act_id = act.as_str().unwrap();
            preflight(
                &ctx,
                &[
                    "--only",
                    "mcp:srv-rem",
                    "--action",
                    act_id,
                    "--agent",
                    agent,
                ],
            )
            .unwrap_or_else(|e| {
                panic!("status 暴露的 agent 级动作 '{agent}:{act_id}' 必须通过预检: {e:#}")
            });
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
    // 直写库模拟历史遗留的非法远程定义（http 非本机回环，保存期校验上线前的存量）
    let mut policy = store::builtin_policy();
    policy["mcp"]["definitions"] = serde_json::json!({
        "remote-only": {"type": "remote", "url": "http://example.com/mcp"}
    });
    let conn = store::open_write(&ctx).unwrap();
    store::write_policy(&conn, &policy).unwrap();
    drop(conn);

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

    preflight(&ctx, &["--only", "mcp:remote-only", "--action", "remove"])
        .expect("无效定义仍应可安全预检 remove");

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

    assert!(
        preflight(
            &ctx,
            &[
                "--only",
                "mcp:disabled-orphan",
                "--action",
                "remove",
                "--agent",
                "claude-code",
            ],
        )
        .is_err(),
        "停用 Agent 的逐 Agent remove 必须被拒绝"
    );

    preflight(
        &ctx,
        &["--only", "mcp:disabled-orphan", "--action", "remove"],
    )
    .expect("孤儿库级 remove 必须通过预检");
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

    // 5. stdio 校验器对远程形态定义 fail-closed（含 url 或遗留 type=="remote"），
    //    以及 stdio 定义携带远程专用 headers 字段必须被拒绝
    assert!(validate_definition_stdio(&json!({"type": "remote"}), "s1").is_err());
    assert!(validate_definition_stdio(&json!({"url": "https://example.com"}), "s1").is_err());
    assert!(validate_definition_stdio(
        &json!({"command": "node", "headers": {"X-A": "${B}"}}),
        "s1"
    )
    .is_err());
}

#[test]
fn mcp_test_validate_definition_remote_matrix() {
    use agent_manager_engine::native_mcp::policy_view::validate_definition;
    use serde_json::json;

    // 1. 合法远程定义：https 任意主机 / 本机回环 http（可带端口与路径），headers 可选
    for def in [
        json!({"url": "https://example.com/mcp"}),
        json!({"url": "https://example.com:8443/mcp?x=1"}),
        json!({"url": "http://localhost/mcp"}),
        json!({"url": "http://localhost:8080/mcp"}),
        json!({"url": "http://127.0.0.1:3000/sse"}),
        json!({"url": "https://example.com/mcp", "headers": {"Authorization": "${A_TOKEN}", "X-Custom-1": "${B_TOKEN}"}}),
        json!({"url": "https://example.com/mcp", "agents": ["claude-code"]}),
    ] {
        assert!(validate_definition(&def, "s1").is_ok(), "应合法: {def}");
    }

    // 2. url 白名单：非 https 且非本机回环一律拒绝，且错误消息不回显 url 正文
    for (def, url_body) in [
        (
            json!({"url": "http://example.com/mcp"}),
            "http://example.com/mcp",
        ),
        (
            json!({"url": "http://localhost.evil.com/mcp"}),
            "http://localhost.evil.com/mcp",
        ),
        (
            json!({"url": "http://192.168.1.1/mcp"}),
            "http://192.168.1.1/mcp",
        ),
        (
            json!({"url": "ftp://example.com/mcp"}),
            "ftp://example.com/mcp",
        ),
        (json!({"url": "file:///etc/passwd"}), "file:///etc/passwd"),
        (json!({"url": "https://"}), "https://"),
        (
            json!({"url": "https://user@example.com/mcp"}),
            "user@example.com",
        ),
        (
            json!({"url": " https://example.com/mcp "}),
            " https://example.com/mcp ",
        ),
    ] {
        let err = validate_definition(&def, "s1").expect_err(&format!("应拒绝: {def}"));
        let msg = err.to_string();
        assert!(msg.contains("url"), "错误消息应指出 url 字段: {msg}");
        assert!(!msg.contains(url_body), "错误消息不得回显 url 正文: {msg}");
    }

    // 3. url 缺失/非字符串、遗留 type=="remote" 缺 url
    for def in [
        json!({"type": "remote"}),
        json!({"url": 123}),
        json!({"url": null}),
    ] {
        let err = validate_definition(&def, "s1").expect_err(&format!("应拒绝: {def}"));
        assert!(err.to_string().contains("url"), "错误消息应指出 url: {err}");
    }

    // 4. 远程与 stdio 字段互斥：command/args/env/startup_timeout_sec 任一出现即拒绝
    for def in [
        json!({"url": "https://example.com/mcp", "command": "node"}),
        json!({"url": "https://example.com/mcp", "args": ["a"]}),
        json!({"url": "https://example.com/mcp", "env": {}}),
        json!({"url": "https://example.com/mcp", "startup_timeout_sec": 30}),
    ] {
        assert!(
            validate_definition(&def, "s1").is_err(),
            "混给应拒绝: {def}"
        );
    }

    // 5. headers 反例：非对象、值非 ${VAR}（含明文 token 形）、非法 header 名、超上限
    for def in [
        json!({"url": "https://example.com/mcp", "headers": "not-object"}),
        json!({"url": "https://example.com/mcp", "headers": {"Authorization": "Bearer plain-token"}}),
        json!({"url": "https://example.com/mcp", "headers": {"Authorization": "A_TOKEN"}}),
        json!({"url": "https://example.com/mcp", "headers": {"Bad Header": "${A}"}}),
        json!({"url": "https://example.com/mcp", "headers": {"": "${A}"}}),
        json!({"url": "https://example.com/mcp", "headers": (0..21).map(|i| (format!("X-H-{i}"), json!("${A}"))).collect::<serde_json::Map<String, serde_json::Value>>()}),
    ] {
        assert!(
            validate_definition(&def, "s1").is_err(),
            "headers 反例应拒绝: {def}"
        );
    }

    // 6. stdio 定义照旧（无 url 无 type==remote 即 stdio 校验）
    assert!(validate_definition(&json!({"command": "node"}), "s1").is_ok());
    assert!(validate_definition(&json!({"command": "node", "url": null}), "s1").is_err());
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

        assert!(
            preflight(
                &ctx,
                &[
                    "--only",
                    "mcp:startup-strict",
                    "--action",
                    "update",
                    "--agent",
                    "codex",
                ],
            )
            .is_err(),
            "非法 timeout 的预检必须 fail-closed: {invalid_json}"
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
    let err_update = preflight_err(
        &ctx,
        &[
            "--only",
            "mcp:srv-allowed",
            "--action",
            "update",
            "--agent",
            "claude-code",
        ],
    );
    assert!(
        err_update.contains("不在服务"),
        "非允许 agent 的 update 必须被 preflight 拒绝: {err_update}"
    );

    // 验证 status 暴露的安全动作均可通过预检
    preflight(
        &ctx,
        &[
            "--only",
            "mcp:srv-allowed",
            "--action",
            "disable",
            "--agent",
            "claude-code",
        ],
    )
    .expect("安全 disable 动作必须通过预检");

    preflight(
        &ctx,
        &[
            "--only",
            "mcp:srv-allowed",
            "--action",
            "remove",
            "--agent",
            "claude-code",
        ],
    )
    .expect("安全 remove 动作必须通过预检");

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
    assert!(
        preflight(
            &ctx,
            &[
                "--only",
                "mcp:srv-allowed",
                "--action",
                "enable",
                "--agent",
                "claude-code",
            ],
        )
        .is_err(),
        "非允许 agent 的 enable 必须被拒绝"
    );

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
    assert!(
        preflight(&ctx, &["--only", "mcp:srv-orp-probe", "--action", "remove"]).is_err(),
        "目标损坏时孤儿 remove 必须被 preflight 拒绝"
    );

    assert!(
        preflight(
            &ctx,
            &[
                "--only",
                "mcp:srv-orp-probe",
                "--action",
                "remove",
                "--agent",
                "claude-code",
            ],
        )
        .is_err(),
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

    // 验证现在可以通过预检并真实 apply
    preflight(&ctx, &["--only", "mcp:srv-orp-probe", "--action", "remove"])
        .expect("修复后孤儿 remove 必须通过预检");

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
fn mcp_test_status_and_preflight_reject_non_regular_targets_before_read() {
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

    assert!(
        preflight(
            &ctx,
            &[
                "--only",
                "mcp:socket-target",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        )
        .is_err(),
        "socket target 的预检必须先拒绝"
    );

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

    assert!(
        preflight(
            &ctx,
            &[
                "--only",
                "mcp:device-target",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        )
        .is_err(),
        "/dev/null device target 的预检必须先拒绝"
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
        .expect_err("--dry-run 已退役，必须在任何写之前被参数解析拒绝");
    assert!(
        dry_err.to_string().contains("已退役 --dry-run"),
        "拒绝消息必须说明 --dry-run 已退役: {dry_err:#}"
    );
    assert!(!ctx.home_dir.exists(), "dry-run 拒绝不得创建 HOME 目录");
    assert!(!ctx.data_dir.exists(), "dry-run 拒绝不得创建 data_dir");
    assert!(!store::db_path(&ctx).exists(), "dry-run 拒绝不得创建 DB");

    let _ = std::fs::remove_dir_all(root);
}

// ---------- 远程 HTTP MCP ----------

/// 为远程测试准备三 codec 环境（含 OpenCode agent 定义）与远程服务定义。
fn setup_remote_ctx(tag: &str, def_value: &str) -> (PathBuf, Context) {
    let (root, ctx) = temp_ctx(tag);
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
    let set_def = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.remote-srv",
                "--value",
                def_value,
            ],
        ),
    );
    assert!(set_def.ok, "远程定义写入必须成功: {:?}", set_def.errors);
    (root, ctx)
}

fn find_element<'v>(st: &'v agent_manager_engine::Envelope, id: &str) -> &'v serde_json::Value {
    assert!(st.ok, "status 必须成功: {:?}", st.errors);
    st.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .unwrap_or_else(|| panic!("必须找到元素 {id}: {}", st.data))
}

#[test]
fn mcp_test_remote_roundtrip_all_codecs() {
    std::env::set_var("AM_TEST_REMOTE_TOKEN", "SENTINEL-REMOTE-TOKEN-3f9a");
    std::env::set_var("AM_TEST_REMOTE_TEAM", "SENTINEL-REMOTE-TEAM-7c21");
    let (root, ctx) = setup_remote_ctx(
        "remote-roundtrip",
        r#"{"url":"https://mcp.example.com/endpoint","headers":{"Authorization":"${AM_TEST_REMOTE_TOKEN}","X-Team":"${AM_TEST_REMOTE_TEAM}"},"agents":["claude-code","codex","opencode"]}"#,
    );
    let claude_path = ctx.home_dir.join(".claude.json");
    let codex_path = ctx.home_dir.join(".codex/config.toml");
    let opencode_path = ctx.home_dir.join(".config/opencode/opencode.json");

    // 1. apply 前 status：三 cell 暴露 add；freshness 恒 not-applicable；
    //    spec.headers 保留 ${VAR} 占位符；observed.headers 只含占位符与布尔。
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:remote-srv");
    assert_eq!(
        el["freshness"]["state"], "not-applicable",
        "远程定义不参与上游探测"
    );
    assert_eq!(el["spec"]["url"], "https://mcp.example.com/endpoint");
    assert_eq!(
        el["spec"]["headers"]["Authorization"],
        "${AM_TEST_REMOTE_TOKEN}"
    );
    assert!(
        el["spec"].get("command").is_none(),
        "远程 spec 不得含 command"
    );
    assert_eq!(
        el["spec"].get("type").map(|v| v.as_str()),
        None,
        "定义不持久化 type"
    );
    let headers_obs = &el["observed"]["headers"];
    assert_eq!(
        headers_obs["Authorization"],
        serde_json::json!({"placeholder": "${AM_TEST_REMOTE_TOKEN}", "resolved": true})
    );
    assert_eq!(
        headers_obs["X-Team"],
        serde_json::json!({"placeholder": "${AM_TEST_REMOTE_TEAM}", "resolved": true})
    );
    for agent in ["claude-code", "codex", "opencode"] {
        let cell = &el["observed"]["agents"][agent];
        assert!(
            cell["actions"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("add")),
            "apply 前 {agent} 必须暴露 add: {cell:?}"
        );
    }

    // 2. apply add 三 codec
    for agent in ["claude-code", "codex", "opencode"] {
        let res = execute(
            &ctx,
            &request(
                "mcp:apply",
                &[
                    "--only",
                    "mcp:remote-srv",
                    "--action",
                    "add",
                    "--agent",
                    agent,
                ],
            ),
        );
        assert!(res.ok, "远程 add {agent} 必须成功: {:?}", res.errors);
    }

    // 3. Claude JSON：type "http" + url + 字面量 ${VAR} headers + 条目级 marker；零明文
    let claude_content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&claude_path).unwrap()).unwrap();
    let claude_srv = &claude_content["mcpServers"]["remote-srv"];
    assert_eq!(claude_srv["type"], "http");
    assert_eq!(claude_srv["url"], "https://mcp.example.com/endpoint");
    assert_eq!(
        claude_srv["headers"]["Authorization"],
        "${AM_TEST_REMOTE_TOKEN}"
    );
    assert_eq!(claude_srv["headers"]["X-Team"], "${AM_TEST_REMOTE_TEAM}");
    assert_eq!(claude_srv["_MANAGED_BY"], "agent-manager");
    for key in ["command", "args", "env"] {
        assert!(claude_srv.get(key).is_none(), "Claude 远程条目不得含 {key}");
    }
    let claude_raw = std::fs::read_to_string(&claude_path).unwrap();
    for sentinel in ["SENTINEL-REMOTE-TOKEN-3f9a", "SENTINEL-REMOTE-TEAM-7c21"] {
        assert!(
            !claude_raw.contains(sentinel),
            "Claude 目标文件零明文: {sentinel}"
        );
    }

    // 4. Codex TOML：url 键即远程 + env_http_headers（header 名 → 变量名）+ 条目级 marker
    let toml_raw = std::fs::read_to_string(&codex_path).unwrap();
    assert!(toml_raw.contains("[mcp_servers.remote-srv]"), "{toml_raw}");
    assert!(
        toml_raw.contains("url = \"https://mcp.example.com/endpoint\""),
        "{toml_raw}"
    );
    assert!(
        toml_raw.contains("_MANAGED_BY = \"agent-manager\""),
        "{toml_raw}"
    );
    assert!(
        toml_raw.contains("[mcp_servers.remote-srv.env_http_headers]"),
        "{toml_raw}"
    );
    assert!(
        toml_raw.contains("Authorization = \"AM_TEST_REMOTE_TOKEN\""),
        "{toml_raw}"
    );
    assert!(
        toml_raw.contains("X-Team = \"AM_TEST_REMOTE_TEAM\""),
        "{toml_raw}"
    );
    assert!(
        !toml_raw.contains("command"),
        "Codex 远程条目不得含 command: {toml_raw}"
    );
    for sentinel in ["SENTINEL-REMOTE-TOKEN-3f9a", "SENTINEL-REMOTE-TEAM-7c21"] {
        assert!(
            !toml_raw.contains(sentinel),
            "Codex 目标文件零明文: {sentinel}"
        );
    }

    // 5. OpenCode JSON：type "remote" + {env:VAR} 插值 headers + 固定 oauth:false + 条目级 marker
    let opencode_content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&opencode_path).unwrap()).unwrap();
    let oc_srv = &opencode_content["mcp"]["remote-srv"];
    assert_eq!(oc_srv["type"], "remote");
    assert_eq!(oc_srv["url"], "https://mcp.example.com/endpoint");
    assert_eq!(
        oc_srv["headers"]["Authorization"],
        "{env:AM_TEST_REMOTE_TOKEN}"
    );
    assert_eq!(oc_srv["headers"]["X-Team"], "{env:AM_TEST_REMOTE_TEAM}");
    assert_eq!(oc_srv["oauth"], false);
    assert_eq!(oc_srv["_MANAGED_BY"], "agent-manager");
    let oc_raw = std::fs::read_to_string(&opencode_path).unwrap();
    for sentinel in ["SENTINEL-REMOTE-TOKEN-3f9a", "SENTINEL-REMOTE-TEAM-7c21"] {
        assert!(
            !oc_raw.contains(sentinel),
            "OpenCode 目标文件零明文: {sentinel}"
        );
    }

    // 6. 回读往返：extract_entry 从条目级识别 marker，digest 一致 → managed-ok
    for (path, format) in [
        (&claude_path, Format::ClaudeJson),
        (&codex_path, Format::CodexToml),
        (&opencode_path, Format::OpenCodeJson),
    ] {
        let (entry, managed) = extract_entry(path, format, "remote-srv").unwrap();
        assert!(managed, "条目级 marker 必须被识别: {}", path.display());
        assert!(entry.is_some());
    }
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:remote-srv");
    for agent in ["claude-code", "codex", "opencode"] {
        assert_eq!(
            el["observed"]["agents"][agent]["state"], "managed-ok",
            "{agent} 回读往返后必须 managed-ok"
        );
    }
    assert_eq!(el["state"], "managed-ok");

    // 7. update：定义加 header 后收敛
    let set_def = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.remote-srv",
                "--value",
                r#"{"url":"https://mcp.example.com/endpoint","headers":{"Authorization":"${AM_TEST_REMOTE_TOKEN}","X-Team":"${AM_TEST_REMOTE_TEAM}","X-New":"${AM_TEST_REMOTE_TOKEN}"},"agents":["claude-code","codex","opencode"]}"#,
            ],
        ),
    );
    assert!(set_def.ok, "{:?}", set_def.errors);
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:remote-srv");
    assert!(
        el["observed"]["agents"]["codex"]["actions"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("update")),
        "定义变更后必须暴露 update"
    );
    let upd = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:remote-srv",
                "--action",
                "update",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(upd.ok, "远程 update 必须成功: {:?}", upd.errors);
    let toml_raw = std::fs::read_to_string(&codex_path).unwrap();
    assert!(
        toml_raw.contains("X-New = \"AM_TEST_REMOTE_TOKEN\""),
        "{toml_raw}"
    );
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:remote-srv");
    assert_eq!(el["observed"]["agents"]["codex"]["state"], "managed-ok");

    // 8. disable → 条目撤下；enable → 重新写入；remove → 彻底清理
    let dis = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:remote-srv",
                "--action",
                "disable",
                "--agent",
                "opencode",
            ],
        ),
    );
    assert!(dis.ok, "{:?}", dis.errors);
    let opencode_content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&opencode_path).unwrap()).unwrap();
    assert!(opencode_content["mcp"].get("remote-srv").is_none());
    let en = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:remote-srv",
                "--action",
                "enable",
                "--agent",
                "opencode",
            ],
        ),
    );
    assert!(en.ok, "{:?}", en.errors);
    let opencode_content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&opencode_path).unwrap()).unwrap();
    assert_eq!(opencode_content["mcp"]["remote-srv"]["type"], "remote");

    for agent in ["claude-code", "codex", "opencode"] {
        let rm = execute(
            &ctx,
            &request(
                "mcp:apply",
                &[
                    "--only",
                    "mcp:remote-srv",
                    "--action",
                    "remove",
                    "--agent",
                    agent,
                ],
            ),
        );
        assert!(rm.ok, "远程 remove {agent} 必须成功: {:?}", rm.errors);
    }
    let claude_content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&claude_path).unwrap()).unwrap();
    assert!(claude_content["mcpServers"].get("remote-srv").is_none());
    let toml_raw = std::fs::read_to_string(&codex_path).unwrap();
    assert!(!toml_raw.contains("remote-srv"), "{toml_raw}");
    let opencode_content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&opencode_path).unwrap()).unwrap();
    assert!(opencode_content["mcp"].get("remote-srv").is_none());

    std::env::remove_var("AM_TEST_REMOTE_TOKEN");
    std::env::remove_var("AM_TEST_REMOTE_TEAM");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_remote_legacy_type_remote_equivalent() {
    std::env::set_var("AM_TEST_REMOTE_LEGACY", "x");
    // 历史遗留 type:"remote" + url 输入与仅含 url 等同处理；type 不持久化进目标
    let (root, ctx) = setup_remote_ctx(
        "remote-legacy-type",
        r#"{"type":"remote","url":"https://legacy.example.com/mcp","headers":{"X-Legacy":"${AM_TEST_REMOTE_LEGACY}"},"agents":["claude-code"]}"#,
    );
    let add = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:remote-srv",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(add.ok, "遗留 type:remote 定义必须可用: {:?}", add.errors);
    let claude_content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ctx.home_dir.join(".claude.json")).unwrap())
            .unwrap();
    let srv = &claude_content["mcpServers"]["remote-srv"];
    assert_eq!(srv["type"], "http", "codec 推导 Claude type 常量");
    assert_eq!(srv["url"], "https://legacy.example.com/mcp");
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:remote-srv");
    assert_eq!(
        el["observed"]["agents"]["claude-code"]["state"],
        "managed-ok"
    );
    std::env::remove_var("AM_TEST_REMOTE_LEGACY");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_remote_headers_rejected_at_policy_set_and_preflight() {
    let (root, ctx) = temp_ctx("remote-headers-reject");

    // 1. policy:set：headers 值非 ${VAR}（含明文 token 形）一律拒绝，消息不回显值
    for (value, leaked) in [
        ("Bearer plain-token-xyz", "plain-token-xyz"),
        ("AM_TEST_ANY_TOKEN", "AM_TEST_ANY_TOKEN"),
        ("", ""),
    ] {
        let def = format!(
            r#"{{"url":"https://example.com/mcp","headers":{{"Authorization":"{value}"}}}}"#
        );
        let res = execute(
            &ctx,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "mcp.definitions.hdr-bad",
                    "--value",
                    &def,
                ],
            ),
        );
        assert!(!res.ok, "headers 值 '{value}' 必须被拒绝");
        assert_eq!(res.errors[0].code, "USAGE_ERROR");
        let msg = &res.errors[0].message;
        assert!(msg.contains("Authorization"), "消息应指出 header 名: {msg}");
        if !leaked.is_empty() {
            assert!(!msg.contains(leaked), "消息不得回显字面值: {msg}");
        }
    }
    // headers 非对象 / 非法 header 名 / 超上限同样拒绝
    for def in [
        r#"{"url":"https://example.com/mcp","headers":"not-object"}"#.to_string(),
        r#"{"url":"https://example.com/mcp","headers":{"Bad Header":"${A}"}}"#.to_string(),
        format!(
            r#"{{"url":"https://example.com/mcp","headers":{{{}}}}}"#,
            (0..21)
                .map(|i| format!(r#""X-H-{i}":"${{A}}""#))
                .collect::<Vec<_>>()
                .join(",")
        ),
    ] {
        let res = execute(
            &ctx,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "mcp.definitions.hdr-bad",
                    "--value",
                    &def,
                ],
            ),
        );
        assert!(!res.ok, "非法 headers 必须被拒绝: {def}");
    }

    // 2. 疑似凭据被 reject_secrets 拦截（SECRET_INPUT_REJECTED 先于 headers 校验）
    let res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.hdr-secret",
                "--value",
                r#"{"url":"https://example.com/mcp","headers":{"Authorization":"ghp_0123456789abcdefABCDEF"}}"#,
            ],
        ),
    );
    assert!(!res.ok, "疑似凭据必须被拒绝");
    assert_eq!(res.errors[0].code, "SECRET_INPUT_REJECTED");
    assert!(!res.errors[0].message.contains("ghp_"), "不得回显凭据正文");

    // 3. preflight：绕过 policy:set 直写库中的明文 header 定义，预检与 apply 必须 fail-closed
    let mut policy = store::builtin_policy();
    policy["mcp"]["definitions"] = serde_json::json!({
        "hdr-literal": {"url": "https://example.com/mcp", "headers": {"X-Key": "literal-value-9f2c"}}
    });
    let conn = store::open_write(&ctx).unwrap();
    store::write_policy(&conn, &policy).unwrap();
    drop(conn);
    let preflight_msgs = preflight_err(
        &ctx,
        &[
            "--only",
            "mcp:hdr-literal",
            "--action",
            "add",
            "--agent",
            "claude-code",
        ],
    );
    assert!(
        !preflight_msgs.contains("literal-value-9f2c"),
        "预检不得回显字面值: {preflight_msgs}"
    );
    let res = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:hdr-literal",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(!res.ok, "mcp:apply 对明文 header 定义必须 fail-closed");
    let msgs = error_messages(&res);
    assert!(
        !msgs.contains("literal-value-9f2c"),
        "mcp:apply 不得回显字面值: {msgs}"
    );
    // status 中该定义 unsupported 且 spec.headers 掩码 <literal>
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:hdr-literal");
    assert_eq!(el["state"], "unsupported");
    assert_eq!(
        el["spec"]["headers"]["X-Key"], "<literal>",
        "字面值必须掩码"
    );
    let serialized = serde_json::to_string(&st.data).unwrap();
    assert!(
        !serialized.contains("literal-value-9f2c"),
        "status 不得泄露字面值"
    );
    assert!(
        !ctx.home_dir.join(".claude.json").exists(),
        "失败链路不得创建目标文件"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_remote_url_whitelist_and_mixed_fields_integration() {
    let (root, ctx) = temp_ctx("remote-url-whitelist");

    // 1a. url 非白名单：policy:set 保存期直接拒绝，消息不回显 url 正文/主机
    let bad_url = "http://internal.example.com/mcp";
    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.bad-url",
                "--value",
                &format!(r#"{{"url":"{bad_url}","agents":["claude-code"]}}"#),
            ],
        ),
    );
    assert!(!set_res.ok, "非法 url 必须在保存期拒绝");
    assert_eq!(set_res.errors[0].code, "USAGE_ERROR");
    let msg = &set_res.errors[0].message;
    assert!(!msg.contains(bad_url), "保存期消息不得回显 url 正文: {msg}");
    assert!(
        !msg.contains("internal.example.com"),
        "保存期消息不得回显主机: {msg}"
    );
    assert!(
        !store::db_path(&ctx).exists(),
        "被拒绝的保存不得初始化数据库"
    );

    // 1b. 历史遗留非法定义（直写库模拟存量）：status unsupported、预检/apply fail-closed 且不回显
    let mut policy = store::builtin_policy();
    policy["mcp"]["definitions"] = serde_json::json!({
        "bad-url": {"url": bad_url, "agents": ["claude-code"]}
    });
    let conn = store::open_write(&ctx).unwrap();
    store::write_policy(&conn, &policy).unwrap();
    drop(conn);
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:bad-url");
    assert_eq!(el["state"], "unsupported");
    assert!(el["observed"]["agents"]["claude-code"]["actions"]
        .as_array()
        .unwrap()
        .is_empty());
    let preflight_msgs = preflight_err(
        &ctx,
        &[
            "--only",
            "mcp:bad-url",
            "--action",
            "add",
            "--agent",
            "claude-code",
        ],
    );
    assert!(
        !preflight_msgs.contains(bad_url),
        "预检消息不得回显 url 正文: {preflight_msgs}"
    );
    assert!(
        !preflight_msgs.contains("internal.example.com"),
        "预检消息不得回显主机: {preflight_msgs}"
    );
    let res = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:bad-url",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(!res.ok, "mcp:apply 必须拒绝非法 url");
    let msgs = error_messages(&res);
    assert!(
        !msgs.contains(bad_url),
        "mcp:apply 消息不得回显 url 正文: {msgs}"
    );
    assert!(
        !msgs.contains("internal.example.com"),
        "mcp:apply 消息不得回显主机: {msgs}"
    );

    // 2a. 远程与 stdio 字段混给（url+command / url+env / url+startup_timeout_sec / url+args）
    //     → policy:set 保存期拒绝，消息说明互斥
    for (name, def) in [
        (
            "mix-command",
            r#"{"url":"https://example.com/mcp","command":"node"}"#,
        ),
        (
            "mix-env",
            r#"{"url":"https://example.com/mcp","env":{"A":"${B}"}}"#,
        ),
        (
            "mix-timeout",
            r#"{"url":"https://example.com/mcp","startup_timeout_sec":30}"#,
        ),
        (
            "mix-args",
            r#"{"url":"https://example.com/mcp","args":["a"]}"#,
        ),
    ] {
        let set_res = execute(
            &ctx,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    &format!("mcp.definitions.{name}"),
                    "--value",
                    def,
                ],
            ),
        );
        assert!(!set_res.ok, "{name} 混给必须在保存期拒绝");
        assert_eq!(set_res.errors[0].code, "USAGE_ERROR");
        let msg = &set_res.errors[0].message;
        assert!(msg.contains("互斥"), "{name} 消息应说明字段互斥: {msg}");
    }

    // 2b. 历史遗留混给定义（直写库模拟存量）：status unsupported、预检 fail-closed
    let mut policy = store::builtin_policy();
    policy["mcp"]["definitions"] = serde_json::json!({
        "mix-legacy": {"url": "https://example.com/mcp", "command": "node", "agents": ["claude-code"]}
    });
    let conn = store::open_write(&ctx).unwrap();
    store::write_policy(&conn, &policy).unwrap();
    drop(conn);
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:mix-legacy");
    assert_eq!(el["state"], "unsupported", "存量混给定义必须 unsupported");
    let err_mixed = preflight_err(
        &ctx,
        &[
            "--only",
            "mcp:mix-legacy",
            "--action",
            "add",
            "--agent",
            "claude-code",
        ],
    );
    assert!(
        err_mixed.contains("互斥"),
        "存量混给定义预检必须 fail-closed: {err_mixed}"
    );

    // 3. 本机回环 http 白名单正向：http://localhost:8080 可全链路 apply
    let (root2, ctx2) = temp_ctx("remote-url-loopback");
    let set_res = execute(
        &ctx2,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.loop-srv",
                "--value",
                r#"{"url":"http://localhost:8080/mcp","agents":["claude-code"]}"#,
            ],
        ),
    );
    assert!(set_res.ok, "{:?}", set_res.errors);
    let add = execute(
        &ctx2,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:loop-srv",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(add.ok, "本机回环 http 必须可用: {:?}", add.errors);
    let claude_content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ctx2.home_dir.join(".claude.json")).unwrap())
            .unwrap();
    assert_eq!(
        claude_content["mcpServers"]["loop-srv"]["url"],
        "http://localhost:8080/mcp"
    );
    assert_eq!(claude_content["mcpServers"]["loop-srv"]["type"], "http");

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(root2);
}

#[test]
fn mcp_test_remote_status_masking_missing_vars_and_secrets_kind() {
    std::env::set_var("AM_TEST_REMOTE_PRESENT", "x");
    std::env::remove_var("AM_TEST_REMOTE_ABSENT");
    let (root, ctx) = setup_remote_ctx(
        "remote-masking",
        r#"{"url":"https://example.com/mcp","headers":{"X-Present":"${AM_TEST_REMOTE_PRESENT}","X-Absent":"${AM_TEST_REMOTE_ABSENT}"},"agents":["claude-code"]}"#,
    );

    // 1. status：observed.headers 只含占位符与布尔；缺失变量进入 blocked_reason 且不暴露 add
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:remote-srv");
    let headers = &el["observed"]["headers"];
    assert_eq!(headers["X-Present"]["resolved"], true);
    assert_eq!(headers["X-Absent"]["resolved"], false);
    assert_eq!(
        headers["X-Absent"]["placeholder"],
        "${AM_TEST_REMOTE_ABSENT}"
    );
    let cell = &el["observed"]["agents"]["claude-code"];
    assert!(
        cell["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("AM_TEST_REMOTE_ABSENT"),
        "缺失 header 变量必须出现在 blocked_reason: {cell:?}"
    );
    assert!(
        !cell["actions"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("add")),
        "缺失 header 变量时不得暴露 add"
    );

    // 2. 预检与 apply 同样被缺失变量 fail-closed 拦截（与 env 同级）
    let preflight_msgs = preflight_err(
        &ctx,
        &[
            "--only",
            "mcp:remote-srv",
            "--action",
            "add",
            "--agent",
            "claude-code",
        ],
    );
    assert!(
        preflight_msgs.contains("AM_TEST_REMOTE_ABSENT"),
        "预检必须被缺失 header 变量拦截: {preflight_msgs}"
    );
    let res = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:remote-srv",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(!res.ok, "mcp:apply 必须被缺失 header 变量拦截");
    assert!(error_messages(&res).contains("AM_TEST_REMOTE_ABSENT"));
    assert!(!ctx.home_dir.join(".claude.json").exists(), "拦截后零写");

    // 3. 变量补齐后 add 成功（与 env 同级语义）
    std::env::set_var("AM_TEST_REMOTE_ABSENT", "SENTINEL-ABSENT-VALUE-51de");
    let add = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:remote-srv",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(add.ok, "变量补齐后 add 必须成功: {:?}", add.errors);

    // 4. secrets:status：headers 占位符纳入且带 kind 字段；变量值绝不外泄
    let sec = execute(&ctx, &request("secrets:status", &[]));
    assert!(sec.ok, "{:?}", sec.errors);
    let variables = sec.data["elements"][0]["observed"]["variables"]
        .as_array()
        .unwrap();
    let header_rows: Vec<_> = variables.iter().filter(|v| v["kind"] == "header").collect();
    assert_eq!(
        header_rows.len(),
        2,
        "headers 必须纳入 secrets:status: {variables:?}"
    );
    assert!(header_rows.iter().all(|v| v["server"] == "remote-srv"));
    assert!(header_rows
        .iter()
        .any(|v| v["env_key"] == "X-Present" && v["resolved"] == true));
    let serialized = serde_json::to_string(&sec.data).unwrap();
    assert!(
        !serialized.contains("SENTINEL-ABSENT-VALUE-51de"),
        "secrets:status 不得外泄变量值: {serialized}"
    );

    std::env::remove_var("AM_TEST_REMOTE_PRESENT");
    std::env::remove_var("AM_TEST_REMOTE_ABSENT");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_remote_external_modification_fail_closed() {
    std::env::set_var("AM_TEST_REMOTE_DRIFT", "x");
    let (root, ctx) = setup_remote_ctx(
        "remote-drift",
        r#"{"url":"https://drift.example.com/mcp","headers":{"X-Key":"${AM_TEST_REMOTE_DRIFT}"},"agents":["claude-code","codex","opencode"]}"#,
    );
    let claude_path = ctx.home_dir.join(".claude.json");
    let codex_path = ctx.home_dir.join(".codex/config.toml");
    let opencode_path = ctx.home_dir.join(".config/opencode/opencode.json");
    for agent in ["claude-code", "codex", "opencode"] {
        let res = execute(
            &ctx,
            &request(
                "mcp:apply",
                &[
                    "--only",
                    "mcp:remote-srv",
                    "--action",
                    "add",
                    "--agent",
                    agent,
                ],
            ),
        );
        assert!(res.ok, "{:?}", res.errors);
    }

    // 1. 外部修改 url → drifted 且暴露 update（digest 涵盖 url）
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&claude_path).unwrap()).unwrap();
    doc["mcpServers"]["remote-srv"]["url"] = serde_json::json!("https://evil.example.com/x");
    std::fs::write(&claude_path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:remote-srv");
    let cell = &el["observed"]["agents"]["claude-code"];
    assert_eq!(cell["state"], "drifted");
    assert!(cell["actions"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("update")));

    // 2. 外部新增未受管字段（OpenCode oauth 属期望集，用 custom_field 反例）→ fail-closed
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&opencode_path).unwrap()).unwrap();
    doc["mcp"]["remote-srv"]["custom_field"] = serde_json::json!(1);
    std::fs::write(&opencode_path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:remote-srv");
    let cell = &el["observed"]["agents"]["opencode"];
    assert_eq!(cell["state"], "drifted");
    assert!(
        cell["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("未受管外部属性"),
        "{cell:?}"
    );
    let oc_bytes = std::fs::read(&opencode_path).unwrap();
    assert!(
        preflight(
            &ctx,
            &[
                "--only",
                "mcp:remote-srv",
                "--action",
                "update",
                "--agent",
                "opencode",
            ],
        )
        .is_err(),
        "预检对未受管外部属性必须 fail-closed"
    );
    let res = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:remote-srv",
                "--action",
                "update",
                "--agent",
                "opencode",
            ],
        ),
    );
    assert!(!res.ok, "mcp:apply 对未受管外部属性必须 fail-closed");
    assert_eq!(
        std::fs::read(&opencode_path).unwrap(),
        oc_bytes,
        "fail-closed 后文件不变"
    );

    // 3. marker 缺失即非受管（Codex 删除条目级 marker 行）→ 拒绝覆盖
    let toml_raw = std::fs::read_to_string(&codex_path).unwrap();
    let stripped: String = toml_raw
        .lines()
        .filter(|line| !line.starts_with("_MANAGED_BY"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&codex_path, format!("{stripped}\n")).unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:remote-srv");
    let cell = &el["observed"]["agents"]["codex"];
    assert_eq!(cell["state"], "drifted");
    assert!(
        cell["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("非受管同名条目"),
        "{cell:?}"
    );
    assert!(
        cell["actions"].as_array().unwrap().is_empty(),
        "非受管冲突不得暴露动作"
    );
    let codex_bytes = std::fs::read(&codex_path).unwrap();
    for action in ["update", "disable", "remove"] {
        assert!(
            preflight(
                &ctx,
                &[
                    "--only",
                    "mcp:remote-srv",
                    "--action",
                    action,
                    "--agent",
                    "codex",
                ],
            )
            .is_err(),
            "预检 {action} 对非受管条目必须 fail-closed"
        );
        let res = execute(
            &ctx,
            &request(
                "mcp:apply",
                &[
                    "--only",
                    "mcp:remote-srv",
                    "--action",
                    action,
                    "--agent",
                    "codex",
                ],
            ),
        );
        assert!(!res.ok, "mcp:apply {action} 对非受管条目必须 fail-closed");
    }
    assert_eq!(
        std::fs::read(&codex_path).unwrap(),
        codex_bytes,
        "fail-closed 后文件不变"
    );

    // 4. headers 值被外部改成明文 → digest 漂移 + 回读仅作 digest 输入，status 输出不含明文
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&claude_path).unwrap()).unwrap();
    doc["mcpServers"]["remote-srv"]["headers"]["X-Key"] =
        serde_json::json!("PLAINTEXT-EXTERNAL-8e4b");
    std::fs::write(&claude_path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    assert!(st.ok);
    let el = find_element(&st, "mcp:remote-srv");
    assert_eq!(el["observed"]["agents"]["claude-code"]["state"], "drifted");
    let serialized = serde_json::to_string(&st.data).unwrap();
    assert!(
        !serialized.contains("PLAINTEXT-EXTERNAL-8e4b"),
        "status 不得回显目标文件明文"
    );

    std::env::remove_var("AM_TEST_REMOTE_DRIFT");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_remote_zero_write_and_unmanaged_marker_conflict() {
    std::env::set_var("AM_TEST_REMOTE_ZW", "x");
    let (root, ctx) = setup_remote_ctx(
        "remote-zero-write",
        r#"{"url":"https://zw.example.com/mcp","headers":{"X-Key":"${AM_TEST_REMOTE_ZW}"},"agents":["claude-code"]}"#,
    );
    let db_bytes = std::fs::read(store::db_path(&ctx)).unwrap();
    let claude_path = ctx.home_dir.join(".claude.json");

    // 1. status / 预检零写：DB 字节不变、不创建目标文件、不创建 lock；
    //    --dry-run 已退役，必须在任何 lock/DB/目标写之前被严格拒绝
    let st = execute(&ctx, &request("mcp:status", &[]));
    assert!(st.ok);
    preflight(
        &ctx,
        &[
            "--only",
            "mcp:remote-srv",
            "--action",
            "add",
            "--agent",
            "claude-code",
        ],
    )
    .expect("远程 add 预检必须成功");
    let dry = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:remote-srv",
                "--action",
                "add",
                "--agent",
                "claude-code",
                "--dry-run",
            ],
        ),
    );
    assert!(
        !dry.ok,
        "mcp:apply --dry-run 已退役，必须被拒绝: {:?}",
        dry.data
    );
    assert!(
        error_messages(&dry).contains("已退役 --dry-run"),
        "拒绝消息必须说明 --dry-run 已退役: {}",
        error_messages(&dry)
    );
    assert!(
        !claude_path.exists(),
        "status/预检/dry-run 拒绝不得创建目标文件"
    );
    assert!(
        !ctx.data_dir.join("agent-manager.apply.lock").exists(),
        "不得创建 lock"
    );
    assert_eq!(
        std::fs::read(store::db_path(&ctx)).unwrap(),
        db_bytes,
        "status/预检/dry-run 拒绝不得改变数据库字节"
    );

    // 2. 外部非受管同名远程条目（无 marker）→ add fail-closed
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    std::fs::write(
        &claude_path,
        r#"{"mcpServers":{"remote-srv":{"type":"http","url":"https://zw.example.com/mcp"}}}"#,
    )
    .unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:remote-srv");
    let cell = &el["observed"]["agents"]["claude-code"];
    assert_eq!(cell["state"], "drifted");
    assert!(cell["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("非受管同名条目"));
    let external_bytes = std::fs::read(&claude_path).unwrap();
    let add = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:remote-srv",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(!add.ok, "外部非受管同名远程条目必须拒绝覆盖");
    assert_eq!(
        std::fs::read(&claude_path).unwrap(),
        external_bytes,
        "外部条目必须原样保留"
    );

    std::env::remove_var("AM_TEST_REMOTE_ZW");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_remote_opencode_type_shape_mismatch_probe_error() {
    let (root, ctx) = setup_remote_ctx(
        "remote-oc-type",
        r#"{"url":"https://oc.example.com/mcp","agents":["opencode"]}"#,
    );
    let opencode_path = ctx.home_dir.join(".config/opencode/opencode.json");
    std::fs::create_dir_all(opencode_path.parent().unwrap()).unwrap();

    // 1. type "local" + url（形态不匹配）→ probe-error
    std::fs::write(
        &opencode_path,
        r#"{"mcp":{"remote-srv":{"type":"local","url":"https://oc.example.com/mcp"}}}"#,
    )
    .unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:remote-srv");
    let cell = &el["observed"]["agents"]["opencode"];
    assert_eq!(cell["state"], "probe-error");
    assert!(cell["blocked_reason"].as_str().unwrap().contains("type"));

    // 2. type "remote" 无 url（走 stdio 形态校验）→ probe-error
    std::fs::write(
        &opencode_path,
        r#"{"mcp":{"remote-srv":{"type":"remote","command":["echo"]}}}"#,
    )
    .unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:remote-srv");
    let cell = &el["observed"]["agents"]["opencode"];
    assert_eq!(cell["state"], "probe-error");
    assert!(cell["blocked_reason"].as_str().unwrap().contains("type"));

    // 3. headers 值非字符串 → probe-error
    std::fs::write(
        &opencode_path,
        r#"{"mcp":{"remote-srv":{"type":"remote","url":"https://oc.example.com/mcp","headers":{"X-K":123},"_MANAGED_BY":"agent-manager","oauth":false}}}"#,
    )
    .unwrap();
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:remote-srv");
    let cell = &el["observed"]["agents"]["opencode"];
    assert_eq!(cell["state"], "probe-error");
    assert!(cell["blocked_reason"].as_str().unwrap().contains("headers"));

    // 4. 远程条目带 stdio 旧位置 env 内 marker 也被识别（兼容）
    std::fs::write(
        &opencode_path,
        r#"{"mcp":{"remote-srv":{"type":"remote","url":"https://oc.example.com/mcp","oauth":false,"environment":{"_MANAGED_BY":"agent-manager"}}}}"#,
    )
    .unwrap();
    let (_entry, managed) =
        extract_entry(&opencode_path, Format::OpenCodeJson, "remote-srv").unwrap();
    assert!(managed, "stdio 旧位置 env 内 marker 必须继续兼容");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_test_policy_set_validates_full_definition_shape() {
    let (root, ctx) = temp_ctx("policy-set-shape");

    // 1. policy:set 直接拒绝：http 非回环 url、url+command 混给、headers 明文值、
    //    遗留 type:remote 缺 url（消息不回显正文）
    for (name, def, leaked) in [
        (
            "shape-http",
            r#"{"url":"http://evil.example.com/x"}"#,
            "evil.example.com",
        ),
        (
            "shape-mixed",
            r#"{"url":"https://example.com/mcp","command":"npx"}"#,
            "https://example.com/mcp",
        ),
        (
            "shape-hdr",
            r#"{"url":"https://example.com/mcp","headers":{"X-Key":"plain-header-value-2b7d"}}"#,
            "plain-header-value-2b7d",
        ),
        ("shape-no-url", r#"{"type":"remote"}"#, ""),
    ] {
        let res = execute(
            &ctx,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    &format!("mcp.definitions.{name}"),
                    "--value",
                    def,
                ],
            ),
        );
        assert!(!res.ok, "{name} 必须在保存期拒绝: {def}");
        assert_eq!(res.errors[0].code, "USAGE_ERROR", "{name}");
        if !leaked.is_empty() {
            assert!(
                !res.errors[0].message.contains(leaked),
                "{name} 消息不得回显正文: {}",
                res.errors[0].message
            );
        }
        let st = execute(&ctx, &request("mcp:status", &[]));
        assert!(st.ok);
        assert!(
            st.data["elements"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["id"] != format!("mcp:{name}")),
            "{name} 被拒绝后不得出现在 status"
        );
    }
    assert!(!store::db_path(&ctx).exists(), "全部被拒后不得初始化数据库");

    // 2. 合法远程定义保存通过且只写库（零动作：不创建目标文件、不产生 binding）
    let set_ok = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.shape-ok",
                "--value",
                r#"{"url":"https://example.com/mcp","headers":{"Authorization":"${AM_TEST_SHAPE_TOKEN}"},"agents":["claude-code"]}"#,
            ],
        ),
    );
    assert!(set_ok.ok, "合法远程定义必须保存成功: {:?}", set_ok.errors);
    assert!(
        !ctx.home_dir.join(".claude.json").exists(),
        "保存定义不得写目标配置文件"
    );
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let has_table: bool = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='mcp_bindings'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);
    if has_table {
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM mcp_bindings", [], |r| r.get(0))
            .unwrap_or(0);
        assert_eq!(rows, 0, "保存定义不得产生分配绑定");
    }
    drop(conn);
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:shape-ok");
    assert_ne!(el["state"], "unsupported", "合法远程定义必须可用");

    // 3. 子字段 patch（deep-merge 粒度）不误伤：对现存定义写 startup_timeout_sec
    //    子键（即便值非法）仍按既有语义保存，由 status/preflight 层拦截
    let set_stdio = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.shape-stdio",
                "--value",
                r#"{"command":"node","args":["s.js"]}"#,
            ],
        ),
    );
    assert!(set_stdio.ok, "{:?}", set_stdio.errors);
    let set_subkey = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.shape-stdio.startup_timeout_sec",
                "--value",
                "\"ten\"",
            ],
        ),
    );
    assert!(
        set_subkey.ok,
        "子字段 patch 保持既有粒度不被误伤: {:?}",
        set_subkey.errors
    );
    let st = execute(&ctx, &request("mcp:status", &[]));
    let el = find_element(&st, "mcp:shape-stdio");
    assert_eq!(el["state"], "unsupported", "非法子字段由 status 层标记");

    // 4. _delete 删除型 patch 不被误伤
    let del = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.shape-ok",
                "--value",
                r#"{"_delete": true}"#,
            ],
        ),
    );
    assert!(del.ok, "_delete patch 必须成功: {:?}", del.errors);
    let st = execute(&ctx, &request("mcp:status", &[]));
    assert!(st.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["id"] != "mcp:shape-ok"));

    // 5. 整表写入：合法远程定义通过；混入非法定义整条拒绝
    let set_map_ok = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions",
                "--value",
                r#"{"_replace":true,"map-remote":{"url":"https://example.com/mcp"},"map-stdio":{"command":"node"}}"#,
            ],
        ),
    );
    assert!(
        set_map_ok.ok,
        "整表合法写入必须成功: {:?}",
        set_map_ok.errors
    );
    let set_map_bad = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions",
                "--value",
                r#"{"_replace":true,"map-bad":{"url":"http://evil.example.com/x"}}"#,
            ],
        ),
    );
    assert!(!set_map_bad.ok, "整表含非法定义必须拒绝");
    assert!(!set_map_bad.errors[0].message.contains("evil.example.com"));

    let _ = std::fs::remove_dir_all(root);
}

// ---------- import_candidates：未受管现有配置发现（只读、掩码） ----------

fn candidate_names(status: &agent_manager_engine::Envelope) -> Vec<(String, String)> {
    status.data["import_candidates"]
        .as_array()
        .expect("import_candidates 必须是数组")
        .iter()
        .map(|c| {
            (
                c["name"].as_str().unwrap().to_string(),
                c["agent"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn candidate_by_name<'a>(
    status: &'a agent_manager_engine::Envelope,
    name: &str,
) -> &'a serde_json::Value {
    status.data["import_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("候选 {} 必须存在", name))
}

#[test]
fn mcp_import_candidates_filters_masks_and_classifies() {
    let (root, ctx) = temp_ctx("import-candidates");

    // 注册 OpenCode agent
    let set_oc = execute(
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
    assert!(set_oc.ok, "{:?}", set_oc.errors);

    // 库内同名定义：文件中的未受管 lib-dup 不得成为候选
    let set_dup = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.lib-dup",
                "--value",
                r#"{"command":"npx","args":["-y","lib-dup-mcp"]}"#,
            ],
        ),
    );
    assert!(set_dup.ok, "{:?}", set_dup.errors);

    // binding 已有记录：apply add bound-one @ claude-code 后物理删除定义 → orphan
    let set_bound = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.bound-one",
                "--value",
                r#"{"command":"echo","args":["bound"],"agents":["claude-code"]}"#,
            ],
        ),
    );
    assert!(set_bound.ok, "{:?}", set_bound.errors);
    let add_bound = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:bound-one",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(add_bound.ok, "{:?}", add_bound.errors);
    let del_bound = execute(
        &ctx,
        &request(
            "policy:delete",
            &["--layer", "local", "--key", "mcp.definitions.bound-one"],
        ),
    );
    assert!(del_bound.ok, "{:?}", del_bound.errors);

    // Claude JSON：覆盖 stdio/远程/受管（条目级与 env 内标记）/库内同名/binding 已知/非法名
    let claude_cfg = ctx.home_dir.join(".claude.json");
    std::fs::write(
        &claude_cfg,
        r#"{
  "mcpServers": {
    "plain-stdio": {"command": "npx", "args": ["-y", "plain-mcp"], "env": {"PLAIN_TOKEN": "sentinel-claude-env-1"}},
    "managed-one": {"command": "x", "env": {"_MANAGED_BY": "agent-manager"}},
    "managed-remote": {"type": "http", "url": "https://example.com/managed", "_MANAGED_BY": "agent-manager"},
    "lib-dup": {"command": "npx", "args": ["lib-dup-mcp"]},
    "bound-one": {"command": "echo", "args": ["bound"]},
    "bad name!": {"command": "npx"},
    "remote-claude": {"type": "http", "url": "https://api.example.com/mcp", "headers": {"Authorization": "sentinel-claude-header-2", "x-api-key": "sentinel-claude-header-3"}}
  }
}"#,
    )
    .unwrap();

    // Codex TOML：stdio env 掩码、env_http_headers 引用直通、静态 http_headers 掩码
    let codex_cfg = ctx.home_dir.join(".codex").join("config.toml");
    std::fs::create_dir_all(codex_cfg.parent().unwrap()).unwrap();
    std::fs::write(
        &codex_cfg,
        r#"[mcp_servers.codex-stdio]
command = "codex-mcp"
args = ["--serve"]

[mcp_servers.codex-stdio.env]
CODEX_KEY = "sentinel-codex-env-4"

[mcp_servers.codex-remote]
url = "https://codex.example.com/mcp"

[mcp_servers.codex-remote.env_http_headers]
Authorization = "REMOTE_TOKEN"

[mcp_servers.codex-remote.http_headers]
x-static = "sentinel-codex-header-5"
"#,
    )
    .unwrap();

    // OpenCode JSON：local command 数组 + environment；remote + headers
    let oc_cfg = ctx
        .home_dir
        .join(".config")
        .join("opencode")
        .join("opencode.json");
    std::fs::create_dir_all(oc_cfg.parent().unwrap()).unwrap();
    std::fs::write(
        &oc_cfg,
        r#"{
  "mcp": {
    "oc-stdio": {"type": "local", "command": ["oc-mcp", "--serve"], "environment": {"OC_KEY": "sentinel-oc-env-6"}},
    "oc-remote": {"type": "remote", "url": "https://oc.example.com/mcp", "headers": {"x-token": "sentinel-oc-header-7"}}
  }
}"#,
    )
    .unwrap();

    let claude_before = std::fs::read_to_string(&claude_cfg).unwrap();
    let codex_before = std::fs::read_to_string(&codex_cfg).unwrap();
    let oc_before = std::fs::read_to_string(&oc_cfg).unwrap();

    let status = execute(&ctx, &request("mcp:status", &[]));
    assert!(status.ok, "{:?}", status.errors);

    // 候选只含未入库、无 binding、名称合法的条目；marker 无 binding（旧体系遗留）
    // 也进入候选（managed-one/managed-remote），marker+binding（bound-one）仍排除
    let mut names = candidate_names(&status);
    names.sort();
    assert_eq!(
        names,
        vec![
            ("codex-remote".to_string(), "codex".to_string()),
            ("codex-stdio".to_string(), "codex".to_string()),
            ("managed-one".to_string(), "claude-code".to_string()),
            ("managed-remote".to_string(), "claude-code".to_string()),
            ("oc-remote".to_string(), "opencode".to_string()),
            ("oc-stdio".to_string(), "opencode".to_string()),
            ("plain-stdio".to_string(), "claude-code".to_string()),
            ("remote-claude".to_string(), "claude-code".to_string()),
        ],
        "库内同名/binding 已知/非法名一律不出现；marker 无 binding 转为候选"
    );

    // previously_managed 标志：marker 无 binding → true；纯外部 → false
    assert_eq!(
        candidate_by_name(&status, "managed-one")["previously_managed"],
        true
    );
    assert_eq!(
        candidate_by_name(&status, "managed-remote")["previously_managed"],
        true
    );
    for name in [
        "plain-stdio",
        "remote-claude",
        "codex-stdio",
        "codex-remote",
        "oc-stdio",
        "oc-remote",
    ] {
        assert_eq!(
            candidate_by_name(&status, name)["previously_managed"],
            false,
            "{name} 无 marker 必须标注 false"
        );
    }

    // 远程/stdio 分类与掩码形态
    let plain = candidate_by_name(&status, "plain-stdio");
    assert_eq!(plain["transport"], "stdio");
    assert_eq!(plain["spec"]["command"], "npx");
    assert_eq!(plain["spec"]["args"][0], "-y");
    assert_eq!(plain["spec"]["env"]["PLAIN_TOKEN"], "<literal>");

    let remote_claude = candidate_by_name(&status, "remote-claude");
    assert_eq!(remote_claude["transport"], "remote");
    assert_eq!(remote_claude["spec"]["url"], "https://api.example.com/mcp");
    assert_eq!(
        remote_claude["spec"]["headers"]["Authorization"],
        "<literal>"
    );
    assert_eq!(remote_claude["spec"]["headers"]["x-api-key"], "<literal>");

    let codex_stdio = candidate_by_name(&status, "codex-stdio");
    assert_eq!(codex_stdio["transport"], "stdio");
    assert_eq!(codex_stdio["spec"]["env"]["CODEX_KEY"], "<literal>");

    // Codex env_http_headers 引用直通 ${VAR}；静态 http_headers 一律 <literal>
    let codex_remote = candidate_by_name(&status, "codex-remote");
    assert_eq!(codex_remote["transport"], "remote");
    assert_eq!(
        codex_remote["spec"]["headers"]["Authorization"],
        "${REMOTE_TOKEN}"
    );
    assert_eq!(codex_remote["spec"]["headers"]["x-static"], "<literal>");

    let oc_stdio = candidate_by_name(&status, "oc-stdio");
    assert_eq!(oc_stdio["transport"], "stdio");
    assert_eq!(oc_stdio["spec"]["command"], "oc-mcp");
    assert_eq!(oc_stdio["spec"]["args"][0], "--serve");
    assert_eq!(oc_stdio["spec"]["env"]["OC_KEY"], "<literal>");
    let oc_remote = candidate_by_name(&status, "oc-remote");
    assert_eq!(oc_remote["transport"], "remote");
    assert_eq!(oc_remote["spec"]["headers"]["x-token"], "<literal>");

    // 哨兵值断言：任何 env/header 明文绝不出现在 status JSON
    let status_json = serde_json::to_string(&status.data).unwrap();
    for sentinel in [
        "sentinel-claude-env-1",
        "sentinel-claude-header-2",
        "sentinel-claude-header-3",
        "sentinel-codex-env-4",
        "sentinel-codex-header-5",
        "sentinel-oc-env-6",
        "sentinel-oc-header-7",
    ] {
        assert!(
            !status_json.contains(sentinel),
            "status JSON 绝不得包含明文 {}",
            sentinel
        );
    }

    // orphan 仍走既有元素路径，不进候选
    let elements = status.data["elements"].as_array().unwrap();
    assert!(elements.iter().any(|e| e["id"] == "mcp:bound-one"));

    // 只读保证：status 后目标文件内容与之前完全一致
    assert_eq!(std::fs::read_to_string(&claude_cfg).unwrap(), claude_before);
    assert_eq!(std::fs::read_to_string(&codex_cfg).unwrap(), codex_before);
    assert_eq!(std::fs::read_to_string(&oc_cfg).unwrap(), oc_before);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_import_candidates_degrades_per_agent_on_missing_or_corrupt_files() {
    let (root, ctx) = temp_ctx("import-degrade");

    // 1. 全部目标文件不存在：候选为空数组，status 仍 ok
    let status = execute(&ctx, &request("mcp:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    assert_eq!(
        status.data["import_candidates"].as_array().unwrap().len(),
        0,
        "无目标文件时候选为空数组"
    );

    // 2. Claude 文件损坏、Codex 文件正常：Claude 候选为空（不报错），Codex 候选仍在
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let claude_cfg = ctx.home_dir.join(".claude.json");
    std::fs::write(&claude_cfg, "{ 这不是合法 JSON").unwrap();
    let codex_cfg = ctx.home_dir.join(".codex").join("config.toml");
    std::fs::create_dir_all(codex_cfg.parent().unwrap()).unwrap();
    std::fs::write(
        &codex_cfg,
        "[mcp_servers.only-codex]\ncommand = \"codex-mcp\"\n",
    )
    .unwrap();

    let status2 = execute(&ctx, &request("mcp:status", &[]));
    assert!(status2.ok, "{:?}", status2.errors);
    assert_eq!(
        candidate_names(&status2),
        vec![("only-codex".to_string(), "codex".to_string())],
        "损坏文件只影响本 agent 候选，不影响其他 agent"
    );

    let _ = std::fs::remove_dir_all(root);
}

/// marker 黑洞修正：有 marker 但无 binding（旧体系遗留）的条目必须作为
/// 导入候选重新可见（previously_managed=true）；marker+binding（真正受管）、
/// 库内同名（即使带 marker）仍排除；无 marker 纯外部候选标注 false。
#[test]
fn mcp_import_candidates_marker_without_binding_is_previously_managed() {
    let (root, ctx) = temp_ctx("import-prev-managed");

    // 库内同名定义：即使目标文件条目带 marker 也不得成为候选
    let set_dup = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.lib-dup",
                "--value",
                r#"{"command":"npx","args":["-y","lib-dup-mcp"]}"#,
            ],
        ),
    );
    assert!(set_dup.ok, "{:?}", set_dup.errors);

    // marker+binding：入库并 add 到 claude-code（apply 写出的盘上条目自带 marker）
    let set_bound = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.bound-marked",
                "--value",
                r#"{"command":"echo","args":["bound"],"agents":["claude-code"]}"#,
            ],
        ),
    );
    assert!(set_bound.ok, "{:?}", set_bound.errors);
    let add_bound = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:bound-marked",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(add_bound.ok, "{:?}", add_bound.errors);

    // 在 apply 写出的 .claude.json 上追加：marker 无 binding（旧体系遗留）、
    // 无 marker（纯外部）、marker 且库内同名三种条目；bound-marked 保持原样
    let claude_cfg = ctx.home_dir.join(".claude.json");
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&claude_cfg).unwrap()).unwrap();
    let servers = doc["mcpServers"].as_object_mut().unwrap();
    servers.insert(
        "legacy-marked".to_string(),
        serde_json::json!({"command": "legacy-mcp", "env": {"_MANAGED_BY": "agent-manager", "LEGACY_TOKEN": "sentinel-legacy-env-1"}}),
    );
    servers.insert(
        "plain-external".to_string(),
        serde_json::json!({"command": "npx", "args": ["-y", "plain-mcp"]}),
    );
    servers.insert(
        "lib-dup".to_string(),
        serde_json::json!({"command": "npx", "args": ["lib-dup-mcp"], "env": {"_MANAGED_BY": "agent-manager"}}),
    );
    std::fs::write(&claude_cfg, serde_json::to_string_pretty(&doc).unwrap()).unwrap();

    // Codex TOML：env 内 marker 无 binding（真实环境 chrome-devtools/grok-search 形态）
    let codex_cfg = ctx.home_dir.join(".codex").join("config.toml");
    std::fs::create_dir_all(codex_cfg.parent().unwrap()).unwrap();
    std::fs::write(
        &codex_cfg,
        r#"[mcp_servers.codex-legacy]
command = "codex-legacy-mcp"

[mcp_servers.codex-legacy.env]
_MANAGED_BY = "agent-manager"
CODEX_LEGACY_KEY = "sentinel-legacy-env-2"
"#,
    )
    .unwrap();

    let status = execute(&ctx, &request("mcp:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    let mut names = candidate_names(&status);
    names.sort();
    assert_eq!(
        names,
        vec![
            ("codex-legacy".to_string(), "codex".to_string()),
            ("legacy-marked".to_string(), "claude-code".to_string()),
            ("plain-external".to_string(), "claude-code".to_string()),
        ],
        "marker+无 binding 与无 marker 进候选；marker+binding、库内同名（带 marker）排除"
    );

    // 标志与掩码：marker 无 binding → previously_managed=true；marker 键不进 env 清单
    let legacy = candidate_by_name(&status, "legacy-marked");
    assert_eq!(legacy["previously_managed"], true);
    assert_eq!(legacy["spec"]["env"]["LEGACY_TOKEN"], "<literal>");
    assert!(
        legacy["spec"]["env"].get("_MANAGED_BY").is_none(),
        "marker 键不得进入候选 env 清单"
    );
    let codex_legacy = candidate_by_name(&status, "codex-legacy");
    assert_eq!(codex_legacy["previously_managed"], true);
    assert_eq!(codex_legacy["spec"]["env"]["CODEX_LEGACY_KEY"], "<literal>");
    assert!(codex_legacy["spec"]["env"].get("_MANAGED_BY").is_none());
    assert_eq!(
        candidate_by_name(&status, "plain-external")["previously_managed"],
        false
    );

    // 哨兵值断言：新可见的旧体系遗留条目的 env 明文绝不进入 status JSON
    let status_json = serde_json::to_string(&status.data).unwrap();
    for sentinel in ["sentinel-legacy-env-1", "sentinel-legacy-env-2"] {
        assert!(
            !status_json.contains(sentinel),
            "status JSON 绝不得包含明文 {sentinel}"
        );
    }

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_import_conversion_definitions_pass_save_validation_without_touching_targets() {
    let (root, ctx) = temp_ctx("import-convert");

    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let claude_cfg = ctx.home_dir.join(".claude.json");
    std::fs::write(
        &claude_cfg,
        r#"{
  "mcpServers": {
    "plain-stdio": {"command": "npx", "args": ["-y", "plain-mcp"], "env": {"PLAIN_TOKEN": "sentinel-env", "lower_key": "sentinel-env-2"}},
    "remote-claude": {"type": "http", "url": "https://api.example.com/mcp", "headers": {"Authorization": "sentinel-header", "x-api-key": "sentinel-header-2"}}
  }
}"#,
    )
    .unwrap();
    let codex_cfg = ctx.home_dir.join(".codex").join("config.toml");
    std::fs::create_dir_all(codex_cfg.parent().unwrap()).unwrap();
    std::fs::write(
        &codex_cfg,
        "[mcp_servers.codex-remote]\nurl = \"https://codex.example.com/mcp\"\n\n[mcp_servers.codex-remote.env_http_headers]\nAuthorization = \"REMOTE_TOKEN\"\n",
    )
    .unwrap();

    let status = execute(&ctx, &request("mcp:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    let claude_before = std::fs::read_to_string(&claude_cfg).unwrap();
    let codex_before = std::fs::read_to_string(&codex_cfg).unwrap();

    // 模拟前端导入转换：env 值一律 ${<ENV_KEY>}；literal header 一律
    // ${<HEADER_NAME大写下划线化>}；${VAR} 引用直通；agents 固定为来源 agent。
    let plain = candidate_by_name(&status, "plain-stdio");
    let env_in = plain["spec"]["env"].as_object().unwrap();
    let mut env_out = serde_json::Map::new();
    for key in env_in.keys() {
        env_out.insert(
            key.clone(),
            serde_json::Value::String(format!("${{{}}}", key)),
        );
    }
    let stdio_def = serde_json::json!({
        "command": plain["spec"]["command"],
        "args": plain["spec"]["args"],
        "env": env_out,
        "agents": [plain["agent"]],
        "_delete": false,
        "_replace": true,
    });
    let set_stdio = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.plain-stdio",
                "--value",
                &stdio_def.to_string(),
            ],
        ),
    );
    assert!(
        set_stdio.ok,
        "env→${{ENV_KEY}} 转换后定义必须过保存校验: {:?}",
        set_stdio.errors
    );

    let header_var = |name: &str| -> String {
        let mut v: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect();
        if v.chars().next().map_or(false, |c| c.is_ascii_digit()) {
            v.insert(0, '_');
        }
        v
    };
    let convert_headers = |cand: &serde_json::Value| -> serde_json::Value {
        let headers_in = cand["spec"]["headers"].as_object().unwrap();
        let mut out = serde_json::Map::new();
        for (name, value) in headers_in {
            let v = value.as_str().unwrap();
            let converted = if v.starts_with("${") && v.ends_with('}') {
                v.to_string()
            } else {
                format!("${{{}}}", header_var(name))
            };
            out.insert(name.clone(), serde_json::Value::String(converted));
        }
        serde_json::json!({
            "url": cand["spec"]["url"],
            "headers": out,
            "agents": [cand["agent"]],
            "_delete": false,
            "_replace": true,
        })
    };

    let remote_def = convert_headers(candidate_by_name(&status, "remote-claude"));
    assert_eq!(remote_def["headers"]["Authorization"], "${AUTHORIZATION}");
    assert_eq!(remote_def["headers"]["x-api-key"], "${X_API_KEY}");
    let set_remote = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.remote-claude",
                "--value",
                &remote_def.to_string(),
            ],
        ),
    );
    assert!(
        set_remote.ok,
        "literal header→${{HEADER_NAME}} 转换后定义必须过保存校验: {:?}",
        set_remote.errors
    );

    let codex_def = convert_headers(candidate_by_name(&status, "codex-remote"));
    assert_eq!(
        codex_def["headers"]["Authorization"], "${REMOTE_TOKEN}",
        "${{VAR}} 引用必须直通"
    );
    assert_eq!(codex_def["agents"][0], "codex");
    let set_codex = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.codex-remote",
                "--value",
                &codex_def.to_string(),
            ],
        ),
    );
    assert!(set_codex.ok, "{:?}", set_codex.errors);

    // 保存不执行：目标文件未被修改
    assert_eq!(std::fs::read_to_string(&claude_cfg).unwrap(), claude_before);
    assert_eq!(std::fs::read_to_string(&codex_cfg).unwrap(), codex_before);

    // 导入后：候选因库内同名被排除，元素进入库视图
    let status2 = execute(&ctx, &request("mcp:status", &[]));
    assert!(status2.ok, "{:?}", status2.errors);
    let names2 = candidate_names(&status2);
    for imported in ["plain-stdio", "remote-claude", "codex-remote"] {
        assert!(
            !names2.iter().any(|(n, _)| n == imported),
            "导入后 {} 不再出现在候选中",
            imported
        );
    }
    let elements2 = status2.data["elements"].as_array().unwrap();
    for imported in ["plain-stdio", "remote-claude", "codex-remote"] {
        assert!(
            elements2
                .iter()
                .any(|e| e["id"] == format!("mcp:{}", imported)),
            "导入后 {} 必须出现在库元素中",
            imported
        );
    }

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_takeover_requires_flag_and_replaces_unmanaged_entry() {
    let (root, ctx) = temp_ctx("takeover");

    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let claude_cfg = ctx.home_dir.join(".claude.json");
    std::fs::write(
        &claude_cfg,
        r#"{
  "globalFlag": true,
  "mcpServers": {
    "takeover-svc": {"command": "old-binary", "args": ["--old"], "env": {"OLD_KEY": "old-secret"}, "custom_field": "will-be-replaced"},
    "other-svc": {"command": "other", "env": {"K": "v"}}
  }
}"#,
    )
    .unwrap();
    let codex_cfg = ctx.home_dir.join(".codex").join("config.toml");
    std::fs::create_dir_all(codex_cfg.parent().unwrap()).unwrap();
    std::fs::write(
        &codex_cfg,
        "[mcp_servers.takeover-svc]\ncommand = \"old-codex\"\n\n[mcp_servers.other-svc]\ncommand = \"other\"\n",
    )
    .unwrap();

    let set_def = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.takeover-svc",
                "--value",
                r#"{"command":"npx","args":["-y","takeover-mcp"],"agents":["claude-code","codex"]}"#,
            ],
        ),
    );
    assert!(set_def.ok, "{:?}", set_def.errors);

    // 1. 无 flag：预检与 apply 一律 fail-closed，文件不变
    let err_no_flag = preflight_err(
        &ctx,
        &[
            "--only",
            "mcp:takeover-svc",
            "--action",
            "add",
            "--agent",
            "claude-code",
        ],
    );
    assert!(
        err_no_flag.contains("非受管同名条目"),
        "无 flag 的预检必须 fail-closed: {err_no_flag}"
    );
    let apply_no_flag = execute(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:takeover-svc",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(!apply_no_flag.ok, "无 flag 的 apply 必须 fail-closed");
    assert!(error_messages(&apply_no_flag).contains("非受管同名条目"));
    let before = std::fs::read_to_string(&claude_cfg).unwrap();
    assert!(before.contains("old-binary"), "拒绝后目标文件必须保持原样");

    // 2. 冲突在 status 中可见（blocked_reason + 不暴露动作）；
    //    携带 --allow-takeover 的 add/update 预检放行（真实替换由第 3 步 apply 读回验证）
    let st_conflict = execute(&ctx, &request("mcp:status", &[]));
    assert!(st_conflict.ok);
    let conflict_cell = &st_conflict.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "mcp:takeover-svc")
        .expect("takeover-svc 必须在 status 中显示")["observed"]["agents"]["claude-code"];
    assert!(
        conflict_cell["blocked_reason"]
            .as_str()
            .unwrap_or("")
            .contains("非受管同名条目"),
        "冲突必须在 status 中可见: {conflict_cell:?}"
    );
    assert!(
        conflict_cell["actions"].as_array().unwrap().is_empty(),
        "未受管冲突单元格不得暴露动作: {conflict_cell:?}"
    );
    preflight(
        &ctx,
        &[
            "--only",
            "mcp:takeover-svc",
            "--action",
            "add",
            "--agent",
            "claude-code",
            "--allow-takeover",
        ],
    )
    .expect("--allow-takeover 的 add 预检必须放行未受管同名条目");

    // 无冲突目标即使带 flag 也不算接管：apply 真实落地且审计 takeover == false
    let set_clean = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.clean-svc",
                "--value",
                r#"{"command":"npx","args":["clean"],"agents":["claude-code"]}"#,
            ],
        ),
    );
    assert!(set_clean.ok, "{:?}", set_clean.errors);
    preflight(
        &ctx,
        &[
            "--only",
            "mcp:clean-svc",
            "--action",
            "add",
            "--agent",
            "claude-code",
            "--allow-takeover",
        ],
    )
    .expect("无冲突目标带 flag 的 add 预检必须通过");
    let apply_clean = execute(
        &ctx,
        &request_with_run(
            "mcp:apply",
            &[
                "--only",
                "mcp:clean-svc",
                "--action",
                "add",
                "--agent",
                "claude-code",
                "--allow-takeover",
            ],
            "takeover-run-clean",
        ),
    );
    assert!(apply_clean.ok, "{:?}", apply_clean.errors);
    let runs_clean = execute(
        &ctx,
        &request_with_run("runs:get", &["--run", "takeover-run-clean"], "audit-clean"),
    );
    assert!(runs_clean.ok, "{:?}", runs_clean.errors);
    let clean_op = runs_clean.data["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["operation_type"] == "mcp:add")
        .expect("必须有 clean-svc 的 mcp:add 审计记录");
    assert_eq!(
        clean_op["detail"]["takeover"], false,
        "无未受管同名条目时审计不得标记 takeover"
    );

    // 3. 有 flag：apply 替换成功——受管渲染落地、marker 出现、其余条目/字段保留
    let apply_flag = execute(
        &ctx,
        &request_with_run(
            "mcp:apply",
            &[
                "--only",
                "mcp:takeover-svc",
                "--action",
                "add",
                "--agent",
                "claude-code",
                "--allow-takeover",
            ],
            "takeover-run-1",
        ),
    );
    assert!(apply_flag.ok, "{:?}", apply_flag.errors);
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&claude_cfg).unwrap()).unwrap();
    assert_eq!(doc["globalFlag"], true, "目标文件其余顶层字段必须保留");
    let other = &doc["mcpServers"]["other-svc"];
    assert_eq!(other["command"], "other");
    assert_eq!(other["env"]["K"], "v", "其余条目必须原样保留");
    let svc = &doc["mcpServers"]["takeover-svc"];
    assert_eq!(svc["command"], "npx");
    assert_eq!(svc["args"][0], "-y");
    assert_eq!(
        svc["env"]["_MANAGED_BY"], "agent-manager",
        "替换后条目必须带受管 marker"
    );
    assert!(
        svc.get("custom_field").is_none() && svc["env"].get("OLD_KEY").is_none(),
        "未受管条目的自有字段随条目整体被受管渲染替换"
    );

    // 4. 审计 detail 记录 takeover: true
    let runs = execute(
        &ctx,
        &request_with_run("runs:get", &["--run", "takeover-run-1"], "audit-check"),
    );
    assert!(runs.ok, "{:?}", runs.errors);
    let ops = runs.data["operations"].as_array().unwrap();
    let apply_op = ops
        .iter()
        .find(|op| op["operation_type"] == "mcp:add")
        .expect("必须有 mcp:add 审计记录");
    assert_eq!(apply_op["detail"]["takeover"], true);

    // 5. Codex TOML 路径：未受管段被受管渲染替换并带条目表级/ env 表 marker，其余服务保留
    let apply_codex = execute(
        &ctx,
        &request_with_run(
            "mcp:apply",
            &[
                "--only",
                "mcp:takeover-svc",
                "--action",
                "add",
                "--agent",
                "codex",
                "--allow-takeover",
            ],
            "takeover-run-2",
        ),
    );
    assert!(apply_codex.ok, "{:?}", apply_codex.errors);
    let toml_text = std::fs::read_to_string(&codex_cfg).unwrap();
    assert!(toml_text.contains("other-svc"), "TOML 其余服务必须保留");
    assert!(toml_text.contains("npx"), "TOML 必须写入受管 command");
    assert!(
        toml_text.contains("_MANAGED_BY = \"agent-manager\""),
        "TOML 替换后必须带受管 marker: {}",
        toml_text
    );
    assert!(!toml_text.contains("old-codex"), "旧未受管内容必须被替换");

    let _ = std::fs::remove_dir_all(root);
}
