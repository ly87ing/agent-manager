use agent_manager_engine::{execute, store, Context, Request};
use fs2::FileExt;
use std::fs::OpenOptions;
use std::path::PathBuf;

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    let base = std::env::temp_dir().join(format!(
        "am-native-rules-test-{}-{}-{:x}",
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
        run_id: "rules-test-run".to_string(),
    }
}

fn request_with_run(command: &str, args: &[&str], run_id: &str) -> Request {
    Request {
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        run_id: run_id.to_string(),
    }
}

fn write_source(ctx: &Context, key: &str, value: &str) {
    let conn = store::open_write(ctx).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS rule_sources (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            revision TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .unwrap();
    let revision = agent_manager_engine::native_mcp::model::sha256_hex(value);
    conn.execute(
        "INSERT INTO rule_sources (key, value, revision, updated_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![key, value, revision, "2026-09-12T00:00:00Z"],
    )
    .unwrap();
}

fn count_rows(ctx: &Context, sql: &str) -> i64 {
    let conn = store::open_readonly(ctx).unwrap().expect("db must exist");
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

#[test]
fn status_succeeds_on_fresh_context_without_creating_anything() {
    let (base, ctx) = temp_ctx("fresh");
    let env = execute(&ctx, &request("rules:status", &["--offline"]));
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.command, "rules.status");
    assert!(env.mode.offline);
    assert!(!env.lock.held);
    let elements = env.data["elements"].as_array().unwrap();
    assert_eq!(elements.len(), 3);
    assert_eq!(elements[0]["id"], "rules:codex_global");
    assert_eq!(elements[1]["id"], "rules:claude_global");
    assert_eq!(elements[2]["id"], "rules:opencode_global");
    for element in elements {
        assert_eq!(element["state"], "missing");
    }

    assert!(!ctx.data_dir.exists(), "status 不得创建 data_dir");
    assert!(!ctx.home_dir.exists(), "status 不得创建 home_dir");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn dot_command_form_is_normalized() {
    let (base, ctx) = temp_ctx("dotform");
    let env = execute(&ctx, &request("rules.status", &[]));
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.command, "rules.status");
    assert_eq!(env.data["domain"], "rules");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn apply_dry_run_is_rejected_without_lock_db_or_target_writes() {
    let (base, ctx) = temp_ctx("dryrun");
    let env = execute(&ctx, &request("rules:apply", &["--dry-run"]));
    assert!(!env.ok, "--dry-run 必须以参数错误拒绝: {:?}", env.data);
    assert_eq!(env.errors[0].code, "RULE_ERROR");
    assert!(env.errors[0].message.contains("参数错误"));
    assert!(!env.lock.held, "被拒绝的 apply 不得持锁");
    assert!(
        !store::db_path(&ctx).exists(),
        "拒绝路径不得创建 DB"
    );
    assert!(
        !ctx.data_dir.join("agent-manager.apply.lock").exists(),
        "拒绝路径不得创建 apply lock"
    );
    assert!(!ctx.home_dir.exists(), "拒绝路径不得写目标");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn real_apply_only_codex_locks_writes_binding_audits_and_preserves_sentinel() {
    let (base, ctx) = temp_ctx("real-apply");
    let marker = "RULES-INTEGRATION-BODY-MARKER-51af";
    write_source(&ctx, "rules.shared", &format!("{marker}\n"));
    std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
    std::fs::write(ctx.home_dir.join(".claude/CLAUDE.md"), "sentinel\n").unwrap();

    let env = execute(
        &ctx,
        &request("rules:apply", &["--only", "rules:codex_global"]),
    );
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.lock.held, "real apply 必须持锁");
    assert_eq!(env.data["domain"], "rules");
    assert_eq!(env.data["steps"].as_array().unwrap().len(), 1);
    assert!(
        !serde_json::to_string(&env).unwrap().contains(marker),
        "envelope 泄漏规则正文"
    );

    let codex_target = ctx.home_dir.join(".codex/AGENTS.md");
    let bytes = std::fs::read(&codex_target).unwrap();
    assert!(
        String::from_utf8_lossy(&bytes).contains(marker),
        "目标文件必须包含渲染正文"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&codex_target)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
    assert_eq!(
        std::fs::read(ctx.home_dir.join(".claude/CLAUDE.md")).unwrap(),
        b"sentinel\n",
        "未选中的 Claude sentinel 不得变化"
    );

    // 随后 status：codex managed-ok，claude 仍 unmanaged（外部 sentinel 无绑定）。
    let status = execute(&ctx, &request("rules:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    let elements = status.data["elements"].as_array().unwrap();
    assert_eq!(elements[0]["state"], "managed-ok");
    assert_eq!(elements[1]["state"], "unmanaged-detected");

    // SQLite：binding 仅 1 行、rules:apply 审计仅 1 条、detail_json 无正文。
    assert_eq!(
        count_rows(&ctx, "SELECT COUNT(*) FROM rule_target_bindings"),
        1
    );
    assert_eq!(
        count_rows(
            &ctx,
            "SELECT COUNT(*) FROM operation_logs WHERE operation_type = 'rules:apply'"
        ),
        1
    );
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let detail: String = conn
        .query_row(
            "SELECT detail_json FROM operation_logs WHERE operation_type = 'rules:apply'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!detail.contains(marker), "detail_json 泄漏规则正文");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn real_apply_rejects_contended_lock_without_touching_target() {
    let (base, ctx) = temp_ctx("lock");
    let lock_path = ctx.data_dir.join("agent-manager.apply.lock");
    std::fs::create_dir_all(&ctx.data_dir).unwrap();
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
                "rules:apply",
                &["--only", "rules:codex_global"],
                "contended-rules",
            ),
        )
    });
    let env = worker.join().unwrap();
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "RULE_ERROR");
    assert!(!env.lock.held);
    assert!(!ctx.home_dir.exists(), "锁竞争失败不得写目标");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn invalid_args_return_rule_error_without_leaking_marker() {
    let (base, ctx) = temp_ctx("invalid");
    let marker = "RULES-ARG-MARKER-e81c";
    for args in [
        vec![
            "--only",
            "rules:codex_global",
            "--only",
            "rules:codex_global",
        ],
        vec!["--only", marker],
        vec!["positional"],
        vec!["--dry-run", "--dry-run"],
    ] {
        let env = execute(
            &ctx,
            &request_with_run("rules:apply", &args, "rules-invalid"),
        );
        assert!(!env.ok, "{args:?} 必须失败");
        assert_eq!(env.errors[0].code, "RULE_ERROR", "{args:?}");
        let serialized = serde_json::to_string(&env).unwrap();
        assert!(
            !serialized.contains(marker),
            "错误 envelope 不得回显传入 token: {args:?}"
        );
    }
    assert!(!ctx.home_dir.exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn apply_with_selected_unmanaged_fails_closed_without_writes() {
    let (base, ctx) = temp_ctx("unmanaged");
    std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    std::fs::write(ctx.home_dir.join(".codex/AGENTS.md"), "external\n").unwrap();

    let env = execute(&ctx, &request("rules:apply", &[]));
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "RULE_ERROR");
    assert!(env.errors[0].message.contains("preflight 拒绝"));
    assert!(!env.lock.held);
    assert!(
        !store::db_path(&ctx).exists(),
        "preflight 失败不得创建 DB"
    );
    assert!(!ctx.data_dir.join("agent-manager.apply.lock").exists());
    assert_eq!(
        std::fs::read(ctx.home_dir.join(".codex/AGENTS.md")).unwrap(),
        b"external\n"
    );

    // 未选中的 unmanaged 不阻塞：--only claude 真实 apply 写入单个目标。
    let env = execute(
        &ctx,
        &request("rules:apply", &["--only", "rules:claude_global"]),
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["steps"].as_array().unwrap().len(), 1);
    assert_eq!(
        env.data["steps"][0]["name"],
        "rules:sync rules:claude_global"
    );
    assert_eq!(
        std::fs::read(ctx.home_dir.join(".codex/AGENTS.md")).unwrap(),
        b"external\n",
        "未选中的外部文件必须原样保留"
    );
    let _ = std::fs::remove_dir_all(&base);
}
