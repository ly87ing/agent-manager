use agent_manager_engine::{execute, store, Context, Request};
use serde_json::Value;
use std::path::PathBuf;

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    // 测试与进程环境隔离：锁状态只由 execute 从 AGENT_MANAGER_LOCK_* 推导，
    // 外部 shell 若带这些变量会污染 lock.held 断言。
    std::env::remove_var("AGENT_MANAGER_LOCK_HELD");
    std::env::remove_var("AGENT_MANAGER_LOCK_PID");
    let base = std::env::temp_dir().join(format!(
        "am-native-system-test-{}-{}-{:x}",
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
        run_id: "system-audit-test-run".to_string(),
    }
}

fn audit(ctx: &Context, args: &[&str]) -> agent_manager_engine::Envelope {
    execute(ctx, &request("system:audit", args))
}

fn section<'a>(env: &'a agent_manager_engine::Envelope, name: &str) -> &'a [Value] {
    env.data["sections"][name]
        .as_array()
        .unwrap_or_else(|| panic!("section '{name}' 必须存在且为数组: {}", env.data))
}

// A. fresh 隔离 Context：结构完整、零写；规则与别名以实际文件为准，没有文件
// （区块）是正常空状态（notice）。整体是否 warning 只取决于 agents 在本机 PATH 下的探测结果。
#[test]
fn fresh_context_audit_is_complete_warning_and_zero_write() {
    let (base, ctx) = temp_ctx("fresh");
    let env = audit(&ctx, &[]);
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.errors.is_empty());
    assert!(!env.lock.held);
    assert!(!env.mode.offline);
    // 3b28f45 起，发布构建会回读编译时身份；隔离 HOME 不应清掉包的来源信息。
    let commit = env!("AGENT_MANAGER_BUILD_COMMIT");
    let date = env!("AGENT_MANAGER_BUILD_DATE");
    let expected_version =
        if commit.is_empty() || chrono::DateTime::parse_from_rfc3339(date).is_err() {
            Value::Null
        } else {
            serde_json::json!({ "commit": commit, "date": date })
        };
    assert_eq!(env.data["version"], expected_version);
    for name in [
        "agents", "skills", "mcp", "rules", "aliases", "secrets", "runtime", "storage",
    ] {
        section(&env, name);
    }
    // tools 域已退役：不再出现在审计 sections 中
    assert!(
        env.data["sections"].get("tools").is_none(),
        "已退役 tools 域不得出现在审计结果: {}",
        env.data
    );
    // aliases 以 rc 文件受管区块为准：fresh 上下文两文件都没有区块 → notice（内置别名只是推荐，
    // 不再是「待生成」的产物）
    let aliases = section(&env, "aliases");
    // Unix 是三个 rc 文件（顺序随用户的 shell：主目标在前）；Windows 是命令脚本目录
    let ids: &[&str] = if cfg!(windows) {
        &["aliases:cmd"]
    } else {
        &["aliases:zshrc", "aliases:zprofile", "aliases:bashrc"]
    };
    assert_eq!(aliases.len(), ids.len());
    for id in ids {
        let item = aliases
            .iter()
            .find(|item| item["name"] == *id)
            .unwrap_or_else(|| panic!("缺 {id}"));
        assert_eq!(item["status"], "notice");
        assert!(item.get("category").is_none());
    }
    // secrets 已接线：fresh 上下文无已启用定义 → 单条 ok（managed-ok 映射）
    let secrets = section(&env, "secrets");
    assert_eq!(secrets.len(), 1);
    assert_eq!(secrets[0]["status"], "ok");
    assert_eq!(secrets[0]["name"], "secrets:mcp-env");
    let summary = &env.data["summary"];
    assert_eq!(summary["errors"], 0);
    let total_warnings: u64 = [
        "agents", "skills", "mcp", "rules", "aliases", "secrets", "runtime", "storage",
    ]
    .iter()
    .map(|name| {
        section(&env, name)
            .iter()
            .filter(|entry| entry["status"] == "warning")
            .count() as u64
    })
    .sum();
    // 规则以实际文件为准：没有文件是正常空状态（notice），不再算「未生成」warning
    for name in ["rules"] {
        let entries = section(&env, name);
        assert_eq!(entries.len(), 2, "{name}");
        for entry in entries {
            assert_eq!(entry["status"], "notice", "{entry}");
            assert!(entry.get("category").is_none(), "{entry}");
        }
    }
    // 其余域在 fresh 上下文里都不产生 warning；只有 agents 可能（取决于 PATH）
    let agent_warnings = section(&env, "agents")
        .iter()
        .filter(|entry| entry["status"] == "warning")
        .count() as u64;
    assert_eq!(total_warnings, agent_warnings);
    assert_eq!(summary["warnings"].as_u64().unwrap(), total_warnings);
    let expected_status = if total_warnings > 0 {
        "warning"
    } else {
        "healthy"
    };
    assert_eq!(env.data["status"], expected_status);
    assert_eq!(summary["warning_categories"]["optional_tools"], 0);
    assert_eq!(summary["warning_categories"]["official_gaps"], 0);
    let categorized: u64 = [
        "agents", "skills", "mcp", "rules", "aliases", "secrets", "runtime", "storage",
    ]
    .iter()
    .map(|name| {
        section(&env, name)
            .iter()
            .filter(|entry| entry["status"] == "warning" && entry.get("category").is_some())
            .count() as u64
    })
    .sum();
    assert_eq!(
        summary["warning_categories"]["unmaterialized"]
            .as_u64()
            .unwrap(),
        categorized
    );
    // runtime/storage 原生状态
    assert_eq!(section(&env, "runtime")[0]["status"], "ok");
    assert_eq!(section(&env, "storage")[0]["status"], "notice");
    // 零写：data/home/DB/log 均不存在
    assert!(!ctx.data_dir.exists(), "audit 不得创建数据目录");
    assert!(!ctx.home_dir.exists(), "audit 不得创建 home 目录");
    assert!(!store::db_path(&ctx).exists());
    let _ = std::fs::remove_dir_all(&base);
}

// B. 已存在的合法 native-manager.db：audit 前后逐字节不变，不初始化缺库/缺表。
#[test]
fn existing_db_is_byte_identical_after_audit() {
    let (base, ctx) = temp_ctx("db-bytes");
    {
        let conn = store::open_write(&ctx).unwrap();
        store::write_policy(&conn, &store::builtin_policy()).unwrap();
    }
    let before = std::fs::read(store::db_path(&ctx)).unwrap();
    let env = audit(&ctx, &[]);
    assert!(env.ok, "{:?}", env.errors);
    let after = std::fs::read(store::db_path(&ctx)).unwrap();
    assert_eq!(before, after, "audit 不得改变数据库字节");
    assert_eq!(section(&env, "storage")[0]["status"], "ok");
    let _ = std::fs::remove_dir_all(&base);
}

// C. 子域状态查询失败只降级该子域：rule_templates 损坏（含 marker）→ rules section 固定
// error，其余 section 仍在；data.status=error；envelope 恰好一条 AUDIT_ERRORS；
// marker/临时路径/原始 DB 错误不外泄；DB 字节不变。库里留着的旧 hook_sources（Hooks 已移除）
// 即便损坏也不影响检查，同样不外泄。
#[test]
fn failing_domain_degrades_only_its_section_without_leaks() {
    let (base, ctx) = temp_ctx("domain-corrupt");
    let marker = "AUDIT-MARKER-9f31c7";
    {
        let conn = store::open_write(&ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE hook_sources (
                key TEXT PRIMARY KEY NOT NULL,
                value_json TEXT NOT NULL,
                revision TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE rule_templates (id TEXT PRIMARY KEY, note TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO hook_sources (key, value_json, revision, updated_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                "hooks.claude",
                format!("{{not-json-{marker}"),
                "rev",
                "2026-09-12T00:00:00Z"
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO rule_templates (id, note) VALUES ('x', ?1)",
            [format!("value_json {marker}")],
        )
        .unwrap();
    }
    let before = std::fs::read(store::db_path(&ctx)).unwrap();
    let env = audit(&ctx, &[]);
    assert!(!env.ok, "损坏子域必须让 envelope ok=false");
    assert_eq!(env.data["status"], "error");
    let rules = section(&env, "rules");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0]["status"], "error");
    for name in ["agents", "skills", "mcp", "runtime", "storage"] {
        assert!(
            section(&env, name)
                .iter()
                .all(|entry| entry["status"] != "error"),
            "section '{name}' 不应被 rules 损坏波及"
        );
    }
    assert!(env.data["sections"].get("hooks").is_none(), "Hooks 已移除");
    let summary = &env.data["summary"];
    assert_eq!(summary["errors"], 1);
    assert_eq!(env.errors.len(), 1);
    assert_eq!(env.errors[0].code, "AUDIT_ERRORS");
    assert!(env.errors[0].message.contains('1'));
    let serialized = serde_json::to_string(&serde_json::json!({
        "data": env.data,
        "warnings": env.warnings.iter().map(|d| (&d.code, &d.message)).collect::<Vec<_>>(),
        "errors": env.errors.iter().map(|d| (&d.code, &d.message)).collect::<Vec<_>>(),
    }))
    .unwrap();
    assert!(
        !serialized.contains(marker),
        "marker 不得外泄: {serialized}"
    );
    assert!(
        !serialized.contains(&base.display().to_string()),
        "真实临时路径不得外泄: {serialized}"
    );
    assert!(
        !serialized.contains("value_json"),
        "原始 DB 错误正文不得外泄: {serialized}"
    );
    let after = std::fs::read(store::db_path(&ctx)).unwrap();
    assert_eq!(before, after);
    let _ = std::fs::remove_dir_all(&base);
}

// E. 参数反例：重复 --offline / 未知 flag / 位置参数均 fail-closed、零写、
// 不回显 token；单个 --offline 成功且 mode.offline=true。
#[test]
fn audit_args_are_strict_and_fail_closed() {
    let (base, ctx) = temp_ctx("args");
    for args in [
        &["--offline", "--offline"][..],
        &["--bogus-flag"][..],
        &["positional-marker"][..],
        &["--offline", "extra"][..],
    ] {
        let env = audit(&ctx, args);
        assert!(!env.ok, "{args:?} 必须失败");
        assert_eq!(env.errors.len(), 1);
        assert_eq!(env.errors[0].code, "USAGE_ERROR");
        assert!(env.data.as_object().unwrap().is_empty());
        let serialized = serde_json::to_string(&env.errors[0].message).unwrap();
        for token in args {
            if *token != "--offline" {
                assert!(!serialized.contains(token), "不得回显 '{token}'");
            }
        }
        assert!(!ctx.data_dir.exists(), "失败路径零写: {args:?}");
    }
    let env = audit(&ctx, &["--offline"]);
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.mode.offline);
    assert!(!ctx.data_dir.exists());
    let _ = std::fs::remove_dir_all(&base);
}

// D 的集成侧：wired 域元素被映射进对应 section（fresh 上下文下 agents 全部
// missing → warning/unmaterialized），notice 不计入 summary。未解析的密钥引用固定带来一条
// warning(unmaterialized)，保证分类诊断在任何 PATH 下都出现。
#[test]
fn wired_domain_elements_map_into_sections() {
    let (base, ctx) = temp_ctx("mapping");
    std::env::remove_var("AM_AUDIT_MAPPING_UNRESOLVED_VAR");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    std::fs::write(
        ctx.home_dir.join(".claude.json"),
        r#"{"mcpServers":{"demo-server":{"command":"d","env":{"DEMO_KEY":"${AM_AUDIT_MAPPING_UNRESOLVED_VAR}"}}}}"#,
    )
    .unwrap();
    let env = audit(&ctx, &[]);
    assert!(env.ok, "{:?}", env.errors);
    let agents = section(&env, "agents");
    assert!(!agents.is_empty());
    for entry in agents {
        // fresh home 下仅允许 missing→warning(unmaterialized) 或 builtin 禁用→notice
        let status = entry["status"].as_str().unwrap();
        assert!(status == "warning" || status == "notice", "{entry}");
        if status == "warning" {
            assert_eq!(entry["category"], "unmaterialized");
        }
        let name = entry["name"].as_str().unwrap();
        assert!(name.starts_with("agents:"), "{name}");
        assert!(entry.get("spec").is_none());
        assert!(entry.get("observed").is_none());
    }
    let unmaterialized = env.data["summary"]["warning_categories"]["unmaterialized"]
        .as_u64()
        .unwrap();
    let agent_warnings = agents
        .iter()
        .filter(|entry| entry["status"] == "warning")
        .count() as u64;
    assert_eq!(section(&env, "secrets")[0]["category"], "unmaterialized");
    assert!(unmaterialized > agent_warnings);
    let expected: u64 = [
        "agents", "skills", "mcp", "rules", "aliases", "secrets", "runtime", "storage",
    ]
    .iter()
    .map(|name| {
        section(&env, name)
            .iter()
            .filter(|entry| entry["status"] == "warning")
            .count() as u64
    })
    .sum();
    assert_eq!(env.data["summary"]["warnings"].as_u64().unwrap(), expected);
    // 分类 warning 非零 → envelope 携带分类诊断（沿用旧语义，仅计数）
    let unmat = env
        .warnings
        .iter()
        .find(|d| d.code == "UNMATERIALIZED")
        .expect("应有 UNMATERIALIZED 分类诊断");
    assert!(unmat.message.contains(&unmaterialized.to_string()));
    let _ = std::fs::remove_dir_all(&base);
}

// secrets 已接线：真实未解析占位符 → secrets section 映射为
// warning(unmaterialized)。
#[test]
fn audit_maps_real_secrets_state() {
    let (base, ctx) = temp_ctx("secrets-wired");
    std::env::remove_var("AM_AUDIT_UNRESOLVED_VAR");
    // 变量行来自助手实际 MCP 条目里的引用（原先来自服务库定义，该模型已退役）。
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    std::fs::write(
        ctx.home_dir.join(".claude.json"),
        r#"{"mcpServers":{"demo-server":{"command":"d","env":{"DEMO_KEY":"${AM_AUDIT_UNRESOLVED_VAR}"}}}}"#,
    )
    .unwrap();
    let env = audit(&ctx, &[]);
    assert!(env.ok, "{:?}", env.errors);
    let secrets = section(&env, "secrets");
    assert_eq!(secrets.len(), 1);
    assert_eq!(secrets[0]["name"], "secrets:mcp-env");
    assert_eq!(secrets[0]["status"], "warning");
    assert_eq!(secrets[0]["category"], "unmaterialized");
    assert!(
        env.data["sections"].get("tools").is_none(),
        "已退役 tools 域不得出现在审计结果: {}",
        env.data
    );
    let _ = std::fs::remove_dir_all(&base);
}
