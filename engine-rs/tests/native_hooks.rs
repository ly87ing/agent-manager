use agent_manager_engine::{execute, store, Context, Request};
use fs2::FileExt;
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::path::PathBuf;

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    let base = std::env::temp_dir().join(format!(
        "am-native-hooks-test-{}-{}-{:x}",
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
        run_id: "hooks-test-run".to_string(),
    }
}

fn request_with_run(command: &str, args: &[&str], run_id: &str) -> Request {
    Request {
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        run_id: run_id.to_string(),
    }
}

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

fn write_hook_source(ctx: &Context, key: &str, value: &Value) {
    let conn = store::open_write(ctx).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS hook_sources (
            key TEXT PRIMARY KEY NOT NULL,
            value_json TEXT NOT NULL,
            revision TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .unwrap();
    let canonical = serde_json::to_string(&canonicalize_value(value)).unwrap();
    let revision = agent_manager_engine::native_mcp::model::sha256_hex(&canonical);
    conn.execute(
        "INSERT INTO hook_sources (key, value_json, revision, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, revision = excluded.revision, updated_at = excluded.updated_at",
        rusqlite::params![key, canonical, revision, "2026-09-12T00:00:00Z"],
    )
    .unwrap();
}

fn write_policy_disabled_agent(ctx: &Context, agent_id: &str) {
    let mut policy = store::builtin_policy();
    policy["agents"][agent_id]["enabled"] = Value::Bool(false);
    let conn = store::open_write(ctx).unwrap();
    store::write_policy(&conn, &policy).unwrap();
}

fn count_rows(ctx: &Context, sql: &str) -> i64 {
    let conn = store::open_readonly(ctx).unwrap().expect("db must exist");
    // 从 SQL 里取表名；表不存在视为 0 行（回滚/未创建场景）。
    let table = sql
        .split_whitespace()
        .skip_while(|w| !w.eq_ignore_ascii_case("from"))
        .nth(1)
        .expect("sql must contain FROM <table>")
        .trim_end_matches(';');
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |r| r.get(0),
        )
        .unwrap();
    if exists == 0 {
        return 0;
    }
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn element(env: &agent_manager_engine::Envelope, id: &str) -> Value {
    env.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("缺元素 {id}"))
}

fn managed_claude_source(command: &str) -> Value {
    json!({
        "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": command}]}]},
        "removed_signatures": [{
            "event": "PreToolUse",
            "matcher": "Write",
            "command_contains": "legacy-hook.sh"
        }]
    })
}

fn managed_codex_source(command: &str) -> Value {
    json!({
        "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": command}]}]}
    })
}

#[test]
fn status_and_plan_on_fresh_context_are_zero_write_with_two_targets() {
    let (base, ctx) = temp_ctx("fresh");
    // 默认来源：claude 只有 removed_signatures、codex hooks 为空 → 两目标 managed-ok 无动作
    let env = execute(&ctx, &request("hooks:status", &["--offline"]));
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.mode.offline);
    assert!(!env.lock.held);
    let elements = env.data["elements"].as_array().unwrap();
    assert_eq!(elements.len(), 2);
    assert_eq!(elements[0]["id"], "hooks:claude");
    assert_eq!(elements[1]["id"], "hooks:codex");
    assert_eq!(elements[0]["spec"]["merge"], "signature-set");
    assert_eq!(elements[0]["spec"]["source_key"], "hooks.claude");
    assert_eq!(elements[0]["spec"]["target"], "~/.claude/settings.json");
    assert_eq!(elements[1]["spec"]["merge"], "whole-file");
    assert_eq!(elements[1]["spec"]["source_key"], "hooks.codex");
    for el in elements {
        assert_eq!(el["state"], "managed-ok");
        assert_eq!(el["actions"], json!([]));
        let observed = &el["observed"];
        for key in [
            "target",
            "target_exists",
            "expected_digest",
            "applied_digest",
            "source_revision",
            "blocked_reason",
            "registration_needed",
            "managed_hook_count",
            "missing_signatures",
            "removed_signatures_present",
        ] {
            assert!(observed.get(key).is_some(), "observed 缺字段 {key}");
        }
        assert_eq!(observed["target_exists"], Value::Bool(false));
        assert_eq!(observed["managed_hook_count"], 0);
    }

    let env = execute(&ctx, &request("hooks:plan", &[]));
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["actions"].as_array().unwrap().len(), 0);
    assert_eq!(env.data["summary"]["write-local"], 0);
    for key in [
        "read",
        "write-local",
        "exec-local",
        "network",
        "install-remove",
    ] {
        assert!(env.data["summary"].get(key).is_some(), "summary 缺 {key}");
    }

    assert!(!ctx.data_dir.exists());
    assert!(!ctx.home_dir.exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn status_strict_args_and_missing_with_managed_hooks_offers_sync() {
    let (base, ctx) = temp_ctx("missing");
    write_hook_source(&ctx, "hooks.claude", &managed_claude_source("echo managed"));
    let env = execute(&ctx, &request("hooks:status", &[]));
    assert!(env.ok, "{:?}", env.errors);
    let claude = element(&env, "hooks:claude");
    assert_eq!(claude["state"], "missing");
    assert_eq!(claude["observed"]["managed_hook_count"], 1);
    assert_eq!(claude["observed"]["missing_signatures"], 1);
    assert_eq!(
        claude["actions"],
        json!([{ "id": "sync", "risk": "write-local", "needs_lock": true, "offline_ok": true }])
    );

    for args in [
        vec!["--only", "hooks:claude"],
        vec!["--dry-run"],
        vec!["positional"],
        vec!["--offline", "--offline"],
    ] {
        let env = execute(&ctx, &request("hooks:status", &args));
        assert!(!env.ok, "status args {args:?} 应失败");
        assert_eq!(env.errors[0].code, "HOOK_ERROR");
    }
    for args in [
        vec!["--only", "hooks:nope"],
        vec!["--only", "hooks:claude", "--only", "hooks:claude"],
        vec!["--only"],
        vec!["--dry-run"],
    ] {
        let env = execute(&ctx, &request("hooks:plan", &args));
        assert!(!env.ok, "plan args {args:?} 应失败");
        assert_eq!(env.errors[0].code, "HOOK_ERROR");
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn apply_dry_run_holds_no_lock_and_creates_nothing() {
    let (base, ctx) = temp_ctx("dryrun");
    write_hook_source(&ctx, "hooks.claude", &managed_claude_source("echo managed"));
    let env = execute(&ctx, &request("hooks:apply", &["--dry-run"]));
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.mode.dry_run);
    assert!(!env.lock.held);
    // 测试夹具写来源时已建库；dry-run 自身不得取锁、不得写目标
    assert!(!ctx.data_dir.join("agent-manager.apply.lock").exists());
    assert!(!ctx.home_dir.exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn claude_first_merge_preserves_user_fields_and_strips_removed_signature() {
    let (base, ctx) = temp_ctx("merge");
    let marker = "HOOKS-MERGE-MARKER-61cf";
    write_hook_source(
        &ctx,
        "hooks.claude",
        &managed_claude_source(&format!("echo {marker}")),
    );
    let settings = ctx.home_dir.join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(
        &settings,
        json!({
            "model": "user-model",
            "hooks": {
                "PreToolUse": [
                    {"matcher": "Write", "hooks": [
                        {"type": "command", "command": "sh legacy-hook.sh"},
                        {"type": "command", "command": "sh keep-user.sh"}
                    ]}
                ],
                "SessionStart": [{"hooks": [{"type": "command", "command": "echo user-own"}]}]
            }
        })
        .to_string(),
    )
    .unwrap();

    let env = execute(&ctx, &request("hooks:apply", &["--only", "hooks:claude"]));
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.lock.held, "real apply 必须持锁");
    assert_eq!(env.data["steps"].as_array().unwrap().len(), 1);

    let merged: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(merged["model"], "user-model", "根级用户字段必须保留");
    let pre = merged["hooks"]["PreToolUse"].as_array().unwrap();
    let commands: Vec<&str> = pre
        .iter()
        .flat_map(|e| e["hooks"].as_array().unwrap().iter())
        .filter_map(|h| h["command"].as_str())
        .collect();
    assert!(commands.contains(&"sh keep-user.sh"), "非托管 hook 保留");
    assert!(
        !commands.contains(&"sh legacy-hook.sh"),
        "removed signature 命中项必须移除"
    );
    let session = merged["hooks"]["SessionStart"].as_array().unwrap();
    let session_commands: Vec<&str> = session
        .iter()
        .flat_map(|e| e["hooks"].as_array().unwrap().iter())
        .filter_map(|h| h["command"].as_str())
        .collect();
    assert!(session_commands.contains(&"echo user-own"));
    assert!(session_commands.contains(&format!("echo {marker}").as_str()));

    #[cfg(unix)]
    {
        // 既有文件保留原 mode（新建文件的 0600 在 only-lock 测试断言）。
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&settings).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
    }

    // status 转 managed-ok；binding 与审计各一条且 detail 无正文
    let status = execute(&ctx, &request("hooks:status", &[]));
    assert_eq!(element(&status, "hooks:claude")["state"], "managed-ok");
    assert_eq!(
        count_rows(&ctx, "SELECT COUNT(*) FROM hook_target_bindings"),
        1
    );
    assert_eq!(
        count_rows(
            &ctx,
            "SELECT COUNT(*) FROM operation_logs WHERE operation_type = 'hooks:apply'"
        ),
        1
    );
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let detail: String = conn
        .query_row(
            "SELECT detail_json FROM operation_logs WHERE operation_type = 'hooks:apply'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!detail.contains(&marker), "审计 detail 不得含 command 正文");
    let serialized = serde_json::to_string(&env).unwrap();
    assert!(!serialized.contains(&marker), "envelope 不得含来源正文");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn codex_foreign_whole_file_without_binding_is_rejected() {
    let (base, ctx) = temp_ctx("codex-foreign");
    write_hook_source(
        &ctx,
        "hooks.codex",
        &managed_codex_source("echo codex-managed"),
    );
    let target = ctx.home_dir.join(".codex/hooks.json");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(
        &target,
        json!({"hooks": {"Other": []}, "foreign": true}).to_string(),
    )
    .unwrap();

    let env = execute(&ctx, &request("hooks:status", &[]));
    let codex = element(&env, "hooks:codex");
    assert_eq!(codex["state"], "unmanaged-detected");
    assert!(codex["observed"]["blocked_reason"].as_str().is_some());
    assert_eq!(codex["actions"], json!([]));

    let env = execute(&ctx, &request("hooks:apply", &["--only", "hooks:codex"]));
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "HOOK_ERROR");
    assert!(!env.lock.held);
    let after = std::fs::read_to_string(&target).unwrap();
    assert!(after.contains("foreign"), "外部内容必须保留");
    assert!(!ctx.home_dir.join(".codex/hooks.json.tmp").exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn codex_semantic_match_without_binding_registers_via_sync() {
    let (base, ctx) = temp_ctx("codex-regneed");
    let source = managed_codex_source("echo codex-managed");
    write_hook_source(&ctx, "hooks.codex", &source);
    let target = ctx.home_dir.join(".codex/hooks.json");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    // 语义相等但字节不同（不同键序/空白）
    std::fs::write(&target, "{ \"hooks\": { \"SessionStart\": [ { \"hooks\": [ { \"type\": \"command\", \"command\": \"echo codex-managed\" } ] } ] } }").unwrap();

    let env = execute(&ctx, &request("hooks:status", &[]));
    let codex = element(&env, "hooks:codex");
    assert_eq!(codex["state"], "drifted");
    assert_eq!(codex["observed"]["registration_needed"], true);
    assert_eq!(codex["actions"][0]["id"], "sync");

    let env = execute(&ctx, &request("hooks:apply", &["--only", "hooks:codex"]));
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        count_rows(
            &ctx,
            "SELECT COUNT(*) FROM hook_target_bindings WHERE target_id='hooks:codex'"
        ),
        1
    );
    let status = execute(&ctx, &request("hooks:status", &[]));
    assert_eq!(element(&status, "hooks:codex")["state"], "managed-ok");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn disabled_target_is_never_probed() {
    let (base, ctx) = temp_ctx("disabled");
    write_policy_disabled_agent(&ctx, "claude-code");
    // claude 目标放在 symlink 祖先之下；若被探测会 unmanaged-detected。
    let real = base.join("real");
    std::fs::create_dir_all(real.join(".claude")).unwrap();
    std::fs::write(real.join(".claude/settings.json"), "{}").unwrap();
    std::os::unix::fs::symlink(&real, &ctx.home_dir).unwrap();

    let env = execute(&ctx, &request("hooks:status", &[]));
    assert!(env.ok, "{:?}", env.errors);
    let claude = element(&env, "hooks:claude");
    assert_eq!(claude["state"], "disabled");
    assert_eq!(claude["observed"]["target_exists"], Value::Null);
    assert_eq!(claude["actions"], json!([]));
    // 对照：enabled 的 codex 目标确实被探测并因祖先符号链接被阻塞。
    let codex = element(&env, "hooks:codex");
    assert_eq!(codex["state"], "unmanaged-detected");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn source_change_produces_drift_with_distinct_digests() {
    let (base, ctx) = temp_ctx("drift");
    write_hook_source(&ctx, "hooks.codex", &managed_codex_source("echo v1"));
    let env = execute(&ctx, &request("hooks:apply", &["--only", "hooks:codex"]));
    assert!(env.ok, "{:?}", env.errors);

    write_hook_source(&ctx, "hooks.codex", &managed_codex_source("echo v2"));
    let env = execute(&ctx, &request("hooks:status", &[]));
    let codex = element(&env, "hooks:codex");
    assert_eq!(codex["state"], "drifted");
    assert_ne!(
        codex["observed"]["expected_digest"],
        codex["observed"]["applied_digest"]
    );

    let env = execute(&ctx, &request("hooks:plan", &["--only", "hooks:codex"]));
    let actions = env.data["actions"].as_array().unwrap();
    assert_eq!(actions.len(), 1);
    let action = &actions[0];
    for key in [
        "op",
        "element",
        "target",
        "risk",
        "before_digest",
        "after_digest",
        "backup_path",
        "cmd",
        "note",
    ] {
        assert!(action.get(key).is_some(), "action 缺字段 {key}");
    }
    assert_eq!(action["op"], "write-file");
    assert_eq!(action["backup_path"], Value::Null);
    assert_eq!(action["cmd"], Value::Null);
    assert_ne!(action["before_digest"], action["after_digest"]);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn external_modification_after_binding_is_rejected_fail_closed() {
    let (base, ctx) = temp_ctx("tamper");
    write_hook_source(&ctx, "hooks.codex", &managed_codex_source("echo v1"));
    execute(&ctx, &request("hooks:apply", &["--only", "hooks:codex"])).unwrap_apply();
    let target = ctx.home_dir.join(".codex/hooks.json");
    std::fs::write(&target, "{\"hooks\":{}}\n").unwrap();

    let env = execute(&ctx, &request("hooks:status", &[]));
    let codex = element(&env, "hooks:codex");
    assert_eq!(codex["state"], "unmanaged-detected");

    let env = execute(&ctx, &request("hooks:apply", &["--only", "hooks:codex"]));
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "HOOK_ERROR");
    assert_eq!(std::fs::read(&target).unwrap(), b"{\"hooks\":{}}\n");
    assert_eq!(
        count_rows(&ctx, "SELECT COUNT(*) FROM hook_target_bindings"),
        1,
        "binding 必须保留"
    );
    let _ = std::fs::remove_dir_all(&base);
}

trait ApplyExt {
    fn unwrap_apply(&self);
}

impl ApplyExt for agent_manager_engine::Envelope {
    fn unwrap_apply(&self) {
        assert!(self.ok, "{:?}", self.errors);
    }
}

#[test]
fn symlink_target_and_bad_json_are_blocked_without_writes() {
    let (base, ctx) = temp_ctx("unsafe");
    write_hook_source(&ctx, "hooks.claude", &managed_claude_source("echo m"));

    // 目标为符号链接
    let elsewhere = base.join("elsewhere.json");
    std::fs::write(&elsewhere, "{}").unwrap();
    let settings = ctx.home_dir.join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &settings).unwrap();
    let env = execute(&ctx, &request("hooks:status", &[]));
    assert_eq!(element(&env, "hooks:claude")["state"], "unmanaged-detected");
    let env = execute(&ctx, &request("hooks:apply", &["--only", "hooks:claude"]));
    assert!(!env.ok);
    assert_eq!(std::fs::read(&elsewhere).unwrap(), b"{}");
    std::fs::remove_file(&settings).unwrap();

    // 坏 JSON
    std::fs::write(&settings, "{not json").unwrap();
    let env = execute(&ctx, &request("hooks:status", &[]));
    let claude = element(&env, "hooks:claude");
    assert_eq!(claude["state"], "unmanaged-detected");
    assert!(claude["observed"]["blocked_reason"].as_str().is_some());
    // plan 整单 fail-closed：选中的 unmanaged 目标不产出半份动作
    let env = execute(&ctx, &request("hooks:plan", &[]));
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "HOOK_ERROR");
    // 未选中不阻塞；plan 与失败的 apply 均不改变 DB（夹具已建库，比对字节）
    let db = store::db_path(&ctx);
    let db_before = std::fs::read(&db).unwrap();
    let env = execute(&ctx, &request("hooks:plan", &["--only", "hooks:codex"]));
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(std::fs::read(&db).unwrap(), db_before, "plan 不得写 DB");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn only_scopes_apply_and_global_lock_is_shared() {
    let (base, ctx) = temp_ctx("only-lock");
    write_hook_source(&ctx, "hooks.claude", &managed_claude_source("echo m"));
    write_hook_source(&ctx, "hooks.codex", &managed_codex_source("echo c"));

    // 精确 --only：只写 codex（新文件 0600）
    let env = execute(&ctx, &request("hooks:apply", &["--only", "hooks:codex"]));
    assert!(env.ok, "{:?}", env.errors);
    let codex_target = ctx.home_dir.join(".codex/hooks.json");
    assert!(codex_target.exists());
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
    assert!(!ctx.home_dir.join(".claude/settings.json").exists());

    // 预独占现有全局锁：real apply 拒绝
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
                "hooks:apply",
                &["--only", "hooks:claude"],
                "contended-hooks",
            ),
        )
    });
    let env = worker.join().unwrap();
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "HOOK_ERROR");
    assert!(!env.lock.held);
    assert!(!ctx.home_dir.join(".claude/settings.json").exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn second_target_binding_failure_rolls_back_both_files_and_db() {
    let (base, ctx) = temp_ctx("rollback-late");
    write_hook_source(&ctx, "hooks.claude", &managed_claude_source("echo m"));
    write_hook_source(&ctx, "hooks.codex", &managed_codex_source("echo c"));
    // 确定性晚失败：合法 binding 表 + 仅对 hooks:codex INSERT RAISE 的 trigger；
    // 两目标都进入 pending，两文件均被原子写入后第二 binding 才失败。
    {
        let conn = store::open_write(&ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE hook_target_bindings (
                target_id TEXT PRIMARY KEY NOT NULL,
                applied_digest TEXT NOT NULL,
                source_revision TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TRIGGER fail_codex_binding BEFORE INSERT ON hook_target_bindings
             WHEN NEW.target_id = 'hooks:codex'
             BEGIN SELECT RAISE(ABORT, 'injected-codex-binding-failure'); END;",
        )
        .unwrap();
    }

    let env = execute(&ctx, &request("hooks:apply", &[]));
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "HOOK_ERROR");
    assert!(env.lock.held, "已进入真实 apply，envelope lock.held=true");
    assert!(
        env.errors[0]
            .message
            .contains("injected-codex-binding-failure"),
        "错误须包含注入失败标记: {}",
        env.errors[0].message
    );
    assert!(
        !ctx.home_dir.join(".claude/settings.json").exists(),
        "第一目标已写文件必须回滚删除"
    );
    assert!(
        !ctx.home_dir.join(".codex/hooks.json").exists(),
        "第二目标已写文件必须回滚删除"
    );
    assert_eq!(
        count_rows(&ctx, "SELECT COUNT(*) FROM hook_target_bindings"),
        0,
        "DB 事务必须整体回滚"
    );
    assert_eq!(
        count_rows(
            &ctx,
            "SELECT COUNT(*) FROM operation_logs WHERE operation_type='hooks:apply'"
        ),
        0
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn audit_failure_rolls_back_files_and_bindings() {
    let (base, ctx) = temp_ctx("rollback-audit");
    write_hook_source(&ctx, "hooks.claude", &managed_claude_source("echo m"));
    // 审计写入触发失败
    {
        let conn = store::open_write(&ctx).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_operation_logs BEFORE INSERT ON operation_logs
             BEGIN SELECT RAISE(ABORT, 'injected-end-failure'); END;",
        )
        .unwrap();
    }
    let env = execute(&ctx, &request("hooks:apply", &["--only", "hooks:claude"]));
    assert!(!env.ok);
    assert!(
        !ctx.home_dir.join(".claude/settings.json").exists(),
        "审计失败后已写文件必须回滚"
    );
    assert_eq!(
        count_rows(&ctx, "SELECT COUNT(*) FROM hook_target_bindings"),
        0
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn corrupt_hook_source_rows_fail_closed_without_leaking_markers() {
    let source_marker = "SOURCE-MARKER-77aa";
    let sha = agent_manager_engine::native_mcp::model::sha256_hex;

    // 辅助：建合法表并插入一行指定 value_json/revision
    let seed = |ctx: &Context, value_json: String, revision: String| {
        let conn = store::open_write(ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE hook_sources (
                key TEXT PRIMARY KEY NOT NULL,
                value_json TEXT NOT NULL,
                revision TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO hook_sources VALUES ('hooks.claude', ?1, ?2, '2026-01-01')",
            rusqlite::params![value_json, revision],
        )
        .unwrap();
    };

    let assert_fail_closed = |ctx: &Context| {
        for command in ["sources:get", "hooks:status", "hooks:plan"] {
            let args: &[&str] = if command == "sources:get" {
                &["--key", "hooks.claude"]
            } else {
                &[]
            };
            let env = execute(ctx, &request_with_run(command, args, "corrupt-source"));
            assert!(!env.ok, "{command} 应 fail-closed");
            let serialized = serde_json::to_string(&env).unwrap();
            assert!(
                !serialized.contains(source_marker),
                "{command} envelope 不得含来源 marker"
            );
            if let Some(log_file) = &env.log_file {
                assert!(!std::fs::read_to_string(log_file)
                    .unwrap()
                    .contains(source_marker));
            }
        }
    };

    // 1) value_json 结构非法（cmd 而非 command）且含 marker
    {
        let (base, ctx) = temp_ctx("corrupt-src-structure");
        let bad =
            json!({"hooks": {"Ev": [{"hooks": [{"type": "command", "cmd": source_marker }]}]}})
                .to_string();
        let rev = sha(&bad);
        seed(&ctx, bad, rev);
        assert_fail_closed(&ctx);
        assert!(!ctx.home_dir.exists());
        let _ = std::fs::remove_dir_all(&base);
    }
    // 2) value_json 非合法 JSON
    {
        let (base, ctx) = temp_ctx("corrupt-src-json");
        seed(&ctx, "{not json".to_string(), sha("x"));
        let env = execute(&ctx, &request("sources:get", &["--key", "hooks.claude"]));
        assert!(!env.ok);
        assert_eq!(env.errors[0].code, "SOURCE_ERROR");
        let env = execute(&ctx, &request("hooks:status", &[]));
        assert!(!env.ok);
        assert_eq!(env.errors[0].code, "HOOK_ERROR");
        let _ = std::fs::remove_dir_all(&base);
    }
    // 3) revision 与 value_json 不一致
    {
        let (base, ctx) = temp_ctx("corrupt-src-revision");
        let good = json!({"hooks": {}}).to_string();
        seed(&ctx, good, sha("other"));
        let env = execute(&ctx, &request("hooks:status", &[]));
        assert!(!env.ok);
        let _ = std::fs::remove_dir_all(&base);
    }
    // 4) 未知 key 行
    {
        let (base, ctx) = temp_ctx("corrupt-src-key");
        let conn = store::open_write(&ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE hook_sources (
                key TEXT PRIMARY KEY NOT NULL,
                value_json TEXT NOT NULL,
                revision TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .unwrap();
        let value = json!({"hooks": {}}).to_string();
        conn.execute(
            "INSERT INTO hook_sources VALUES (?1, ?2, ?3, '2026-01-01')",
            rusqlite::params![format!("hooks.unknown-{source_marker}"), value, sha("x")],
        )
        .unwrap();
        drop(conn);
        let env = execute(&ctx, &request("hooks:status", &[]));
        assert!(!env.ok);
        assert!(!serde_json::to_string(&env).unwrap().contains(source_marker));
        let _ = std::fs::remove_dir_all(&base);
    }
    // 5) 坏 schema（3 列）
    {
        let (base, ctx) = temp_ctx("corrupt-src-schema");
        let conn = store::open_write(&ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE hook_sources (
                key TEXT PRIMARY KEY NOT NULL,
                value_json TEXT NOT NULL,
                revision TEXT NOT NULL
            )",
        )
        .unwrap();
        drop(conn);
        let env = execute(&ctx, &request("hooks:status", &[]));
        assert!(!env.ok);
        assert_eq!(env.errors[0].code, "HOOK_ERROR");
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[test]
fn corrupt_binding_rows_fail_closed_without_leaking_markers() {
    let (base, ctx) = temp_ctx("corrupt-binding");
    let binding_marker = "BINDING-MARKER-88bb";
    {
        let conn = store::open_write(&ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE hook_target_bindings (
                target_id TEXT PRIMARY KEY NOT NULL,
                applied_digest TEXT NOT NULL,
                source_revision TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO hook_target_bindings VALUES (?1, ?2, ?3, '2026-01-01')",
            rusqlite::params![
                format!("hooks:claude-{binding_marker}"),
                agent_manager_engine::native_mcp::model::sha256_hex("a"),
                agent_manager_engine::native_mcp::model::sha256_hex("b")
            ],
        )
        .unwrap();
    }
    for command in ["hooks:status", "hooks:plan"] {
        let env = execute(&ctx, &request_with_run(command, &[], "corrupt-binding"));
        assert!(!env.ok, "{command} 应 fail-closed");
        assert_eq!(env.errors[0].code, "HOOK_ERROR");
        let serialized = serde_json::to_string(&env).unwrap();
        assert!(
            !serialized.contains(binding_marker),
            "{command} envelope 不得含绑定行 marker"
        );
        if let Some(log_file) = &env.log_file {
            assert!(!std::fs::read_to_string(log_file)
                .unwrap()
                .contains(binding_marker));
        }
    }
    assert!(!ctx.home_dir.exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn invalid_target_hooks_subtree_is_unmanaged_and_plan_apply_refuse() {
    let (base, ctx) = temp_ctx("bad-subtree");
    write_hook_source(&ctx, "hooks.claude", &managed_claude_source("echo m"));
    let settings = ctx.home_dir.join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();

    for (tag, content) in [
        (
            "event-not-array",
            json!({"hooks": {"SessionStart": {"hooks": []}}}).to_string(),
        ),
        (
            "entry-not-object",
            json!({"hooks": {"SessionStart": ["x"]}}).to_string(),
        ),
        (
            "hooks-not-array",
            json!({"hooks": {"SessionStart": [{"hooks": {}}]}}).to_string(),
        ),
        (
            "hook-missing-command",
            json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command"}]}]}}).to_string(),
        ),
        (
            "hook-type-not-command",
            json!({"hooks": {"SessionStart": [{"hooks": [{"type": "prompt", "command": "x"}]}]}})
                .to_string(),
        ),
        (
            "hook-empty-command",
            json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": " "}]}]}})
                .to_string(),
        ),
        (
            "empty-event-array",
            json!({"hooks": {"SessionStart": []}}).to_string(),
        ),
        (
            "empty-entry-hooks",
            json!({"hooks": {"SessionStart": [{"hooks": []}]}}).to_string(),
        ),
        (
            "empty-event-name",
            json!({"hooks": {"": [{"hooks": [{"type": "command", "command": "x"}]}]}}).to_string(),
        ),
        (
            "blank-event-name",
            json!({"hooks": {"  ": [{"hooks": [{"type": "command", "command": "x"}]}]}})
                .to_string(),
        ),
    ] {
        std::fs::write(&settings, &content).unwrap();
        let env = execute(&ctx, &request("hooks:status", &[]));
        let claude = element(&env, "hooks:claude");
        assert_eq!(claude["state"], "unmanaged-detected", "{tag}");
        assert_eq!(claude["actions"], json!([]), "{tag}");
        let env = execute(&ctx, &request("hooks:plan", &["--only", "hooks:claude"]));
        assert!(!env.ok, "{tag} plan 应拒绝");
        let env = execute(&ctx, &request("hooks:apply", &["--only", "hooks:claude"]));
        assert!(!env.ok, "{tag} apply 应拒绝");
        assert_eq!(
            std::fs::read(&settings).unwrap(),
            content.as_bytes(),
            "{tag} 目标不变"
        );
    }
    // 合法但有未知附加字段的文件仍可安全合并
    std::fs::write(
        &settings,
        json!({"custom_root": true, "hooks": {"SessionStart": [{"matcher": "Write", "extra_field": 1, "hooks": [{"type": "command", "command": "echo user", "timeout": 5}]}]}}).to_string(),
    )
    .unwrap();
    let env = execute(&ctx, &request("hooks:status", &[]));
    assert_eq!(element(&env, "hooks:claude")["state"], "drifted");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn plan_before_digest_uses_actual_bytes_for_unbound_merge() {
    let (base, ctx) = temp_ctx("before-digest");
    write_hook_source(
        &ctx,
        "hooks.claude",
        &managed_claude_source("echo new-managed"),
    );
    let settings = ctx.home_dir.join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let existing = json!({"model": "m", "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "echo user"}]}]}});
    std::fs::write(
        &settings,
        format!(
            "{}
",
            serde_json::to_string_pretty(&existing).unwrap()
        ),
    )
    .unwrap();
    let old_sha = agent_manager_engine::native_mcp::model::sha256_hex_bytes(
        &std::fs::read(&settings).unwrap(),
    );

    let env = execute(&ctx, &request("hooks:plan", &["--only", "hooks:claude"]));
    assert!(env.ok, "{:?}", env.errors);
    let actions = env.data["actions"].as_array().unwrap();
    assert_eq!(actions.len(), 1);
    let action = &actions[0];
    assert_eq!(
        action["before_digest"].as_str().unwrap(),
        old_sha,
        "before=旧字节 sha"
    );
    let after = action["after_digest"].as_str().unwrap();
    assert_ne!(after, old_sha, "after=渲染 sha 且与 before 不同");

    // status observed.actual_digest 与实际字节一致
    let env = execute(&ctx, &request("hooks:status", &[]));
    assert_eq!(
        element(&env, "hooks:claude")["observed"]["actual_digest"]
            .as_str()
            .unwrap(),
        old_sha
    );
    assert_eq!(
        element(&env, "hooks:codex")["observed"]["actual_digest"],
        Value::Null
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn invalid_args_return_hook_error_without_echoing_values() {
    let (base, ctx) = temp_ctx("invalid-args");
    let marker = "HOOKS-ARG-MARKER-c391";
    for args in [
        vec!["--only", marker],
        vec!["--bogus"],
        vec!["positional"],
        vec!["--dry-run", "--dry-run"],
    ] {
        let env = execute(
            &ctx,
            &request_with_run("hooks:apply", &args, "hooks-invalid"),
        );
        assert!(!env.ok, "{args:?} 应失败");
        assert_eq!(env.errors[0].code, "HOOK_ERROR");
        assert!(!serde_json::to_string(&env).unwrap().contains(marker));
        if let Some(log_file) = &env.log_file {
            assert!(!std::fs::read_to_string(log_file).unwrap().contains(marker));
        }
    }
    assert!(!ctx.data_dir.join("agent-manager.apply.lock").exists());
    assert!(!ctx.home_dir.exists());
    let _ = std::fs::remove_dir_all(&base);
}

// ---------- --allow-takeover：显式接管（仅 --only 单目标） ----------

fn audit_takeover_flags(ctx: &Context) -> Vec<bool> {
    let conn = store::open_readonly(ctx).unwrap().expect("db must exist");
    let mut stmt = conn
        .prepare("SELECT detail_json FROM operation_logs WHERE operation_type = 'hooks:apply'")
        .unwrap();
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<String>, _>>()
        .unwrap();
    rows.iter()
        .map(|detail| {
            let value: Value = serde_json::from_str(detail).unwrap();
            value["takeover"]
                .as_bool()
                .unwrap_or_else(|| panic!("审计 detail 必须恒含 takeover 布尔字段: {detail}"))
        })
        .collect()
}

#[test]
fn allow_takeover_flag_requires_exactly_one_only_target() {
    let (base, ctx) = temp_ctx("takeover-args");
    // 缺 --only（批量缺省）、多个 --only、重复 flag：一律拒绝
    for (command, args) in [
        ("hooks:plan", vec!["--allow-takeover"]),
        ("hooks:apply", vec!["--allow-takeover"]),
        (
            "hooks:plan",
            vec![
                "--only",
                "hooks:claude",
                "--only",
                "hooks:codex",
                "--allow-takeover",
            ],
        ),
        (
            "hooks:apply",
            vec![
                "--only",
                "hooks:codex",
                "--allow-takeover",
                "--allow-takeover",
            ],
        ),
        ("hooks:status", vec!["--allow-takeover"]),
    ] {
        let env = execute(&ctx, &request(command, &args));
        assert!(!env.ok, "{command} {args:?} 应失败");
        assert_eq!(env.errors[0].code, "HOOK_ERROR");
        assert!(
            env.errors[0].message.contains("参数错误"),
            "{command} {args:?} 必须是参数错误: {}",
            env.errors[0].message
        );
    }
    // 合法形态：单目标 + flag 通过参数解析（fresh ctx 两目标 managed-ok 无动作）
    for command in ["hooks:plan", "hooks:apply"] {
        let mut args = vec!["--only", "hooks:codex", "--allow-takeover"];
        if command == "hooks:apply" {
            args.push("--dry-run");
        }
        let env = execute(&ctx, &request(command, &args));
        assert!(env.ok, "{command} {args:?} 应通过: {:?}", env.errors);
    }
    assert!(!ctx.home_dir.exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn codex_foreign_whole_file_takeover_with_flag_replaces_and_marks_plan_and_audit() {
    let (base, ctx) = temp_ctx("codex-takeover");
    write_hook_source(
        &ctx,
        "hooks.codex",
        &managed_codex_source("echo codex-managed-takeover"),
    );
    let target = ctx.home_dir.join(".codex/hooks.json");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let foreign = json!({
        "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "sh user-own-session.sh"}]}]},
        "foreign": true
    });
    std::fs::write(
        &target,
        format!("{}\n", serde_json::to_string_pretty(&foreign).unwrap()),
    )
    .unwrap();
    let foreign_sha =
        agent_manager_engine::native_mcp::model::sha256_hex_bytes(&std::fs::read(&target).unwrap());

    // 无 flag：plan 与 apply 维持 fail-closed（与此前行为完全一致）
    let env = execute(&ctx, &request("hooks:plan", &["--only", "hooks:codex"]));
    assert!(!env.ok, "无 flag plan 必须拒绝");
    let env = execute(&ctx, &request("hooks:apply", &["--only", "hooks:codex"]));
    assert!(!env.ok, "无 flag apply 必须拒绝");
    assert!(!env.lock.held);

    // 有 flag：plan 放行并标注 takeover + whole-file 风险说明
    let env = execute(
        &ctx,
        &request("hooks:plan", &["--only", "hooks:codex", "--allow-takeover"]),
    );
    assert!(env.ok, "{:?}", env.errors);
    let actions = env.data["actions"].as_array().unwrap();
    assert_eq!(actions.len(), 1);
    let action = &actions[0];
    assert_eq!(
        action["takeover"], true,
        "codex whole-file 替换必须标注 takeover"
    );
    assert!(
        action["note"].as_str().unwrap().contains("whole-file 替换")
            && action["note"]
                .as_str()
                .unwrap()
                .contains("并入 hooks.codex 来源"),
        "note 必须明示整体替换与先并来源建议: {}",
        action["note"]
    );
    assert_eq!(action["before_digest"].as_str().unwrap(), foreign_sha);
    assert_ne!(action["after_digest"].as_str().unwrap(), foreign_sha);

    // apply：整文件替换外部内容，登记 binding，审计 takeover=true
    let env = execute(
        &ctx,
        &request(
            "hooks:apply",
            &["--only", "hooks:codex", "--allow-takeover"],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
    let after: Value = serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(after.get("foreign"), None, "外部根字段必须被整体替换");
    let session = after["hooks"]["SessionStart"].as_array().unwrap();
    let commands: Vec<&str> = session
        .iter()
        .flat_map(|e| e["hooks"].as_array().unwrap().iter())
        .filter_map(|h| h["command"].as_str())
        .collect();
    assert!(commands.contains(&"echo codex-managed-takeover"));
    assert!(
        !commands.contains(&"sh user-own-session.sh"),
        "外部 hook 必须被整体替换"
    );
    assert_eq!(
        count_rows(
            &ctx,
            "SELECT COUNT(*) FROM hook_target_bindings WHERE target_id='hooks:codex'"
        ),
        1
    );
    assert_eq!(audit_takeover_flags(&ctx), vec![true]);

    // status 转 managed-ok
    let status = execute(&ctx, &request("hooks:status", &[]));
    assert_eq!(element(&status, "hooks:codex")["state"], "managed-ok");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn claude_external_modification_takeover_merges_and_preserves_user_fields() {
    let (base, ctx) = temp_ctx("claude-takeover");
    write_hook_source(
        &ctx,
        "hooks.claude",
        &managed_claude_source("echo managed-takeover"),
    );
    let settings = ctx.home_dir.join(".claude/settings.json");
    execute(&ctx, &request("hooks:apply", &["--only", "hooks:claude"])).unwrap_apply();

    // 绑定后外部修改：加用户根字段与用户自有 hook
    let mut current: Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    current["model"] = json!("user-model");
    current["hooks"]["SessionStart"]
        .as_array_mut()
        .unwrap()
        .push(json!({"hooks": [{"type": "command", "command": "echo user-own"}]}));
    std::fs::write(&settings, serde_json::to_string_pretty(&current).unwrap()).unwrap();

    // 无 flag：plan 与 apply 维持拒绝
    let env = execute(&ctx, &request("hooks:plan", &["--only", "hooks:claude"]));
    assert!(!env.ok, "无 flag plan 必须拒绝外部修改目标");
    let env = execute(&ctx, &request("hooks:apply", &["--only", "hooks:claude"]));
    assert!(!env.ok, "无 flag apply 必须拒绝外部修改目标");

    // 有 flag：signature-set 合并，动作不标 takeover（合并天然保留用户内容）
    let env = execute(
        &ctx,
        &request(
            "hooks:plan",
            &["--only", "hooks:claude", "--allow-takeover"],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
    let actions = env.data["actions"].as_array().unwrap();
    assert_eq!(actions.len(), 1);
    assert!(
        actions[0].get("takeover").is_none(),
        "claude 合并不标 takeover"
    );

    let env = execute(
        &ctx,
        &request(
            "hooks:apply",
            &["--only", "hooks:claude", "--allow-takeover"],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
    let merged: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(merged["model"], "user-model", "根级用户字段必须保留");
    let session = merged["hooks"]["SessionStart"].as_array().unwrap();
    let commands: Vec<&str> = session
        .iter()
        .flat_map(|e| e["hooks"].as_array().unwrap().iter())
        .filter_map(|h| h["command"].as_str())
        .collect();
    assert!(commands.contains(&"echo user-own"), "用户自有 hook 保留");
    assert!(
        commands.contains(&"echo managed-takeover"),
        "受管 hook 合入"
    );

    // 两次 apply 的审计：第一次无 flag takeover=false，第二次 takeover=true
    assert_eq!(audit_takeover_flags(&ctx), vec![false, true]);
    let status = execute(&ctx, &request("hooks:status", &[]));
    assert_eq!(element(&status, "hooks:claude")["state"], "managed-ok");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn allow_takeover_does_not_bypass_structural_or_batch_fail_closed() {
    let (base, ctx) = temp_ctx("takeover-hard-block");
    write_hook_source(
        &ctx,
        "hooks.codex",
        &managed_codex_source("echo codex-managed"),
    );
    let target = ctx.home_dir.join(".codex/hooks.json");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    // 目标 JSON 损坏：探测即阻塞（actual_digest 为空），flag 也不得放行
    std::fs::write(&target, "{not json").unwrap();

    let env = execute(
        &ctx,
        &request("hooks:plan", &["--only", "hooks:codex", "--allow-takeover"]),
    );
    assert!(!env.ok, "结构损坏目标即使带 flag 也必须拒绝");
    let env = execute(
        &ctx,
        &request(
            "hooks:apply",
            &["--only", "hooks:codex", "--allow-takeover"],
        ),
    );
    assert!(!env.ok, "结构损坏目标即使带 flag 也必须拒绝");
    assert!(!env.lock.held);
    assert_eq!(std::fs::read(&target).unwrap(), b"{not json");

    // 批量场景：claude 正常、codex 外部内容阻塞；无 flag 的批量 plan 仍整体拒绝
    std::fs::write(&target, json!({"hooks": {}, "foreign": true}).to_string()).unwrap();
    let env = execute(&ctx, &request("hooks:plan", &[]));
    assert!(!env.ok, "批量 plan 对阻塞目标仍整体 fail-closed");
    let _ = std::fs::remove_dir_all(&base);
}
