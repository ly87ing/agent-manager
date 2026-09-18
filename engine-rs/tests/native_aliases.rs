use agent_manager_engine::{execute, store, Context, Request};
use fs2::FileExt;
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::path::PathBuf;

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    // 锁状态只由 execute 从 AGENT_MANAGER_LOCK_* 推导，外部环境变量不得污染断言。
    std::env::remove_var("AGENT_MANAGER_LOCK_HELD");
    std::env::remove_var("AGENT_MANAGER_LOCK_PID");
    let base = std::env::temp_dir().join(format!(
        "am-native-aliases-test-{}-{}-{:x}",
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
        run_id: "aliases-test-run".to_string(),
    }
}

fn request_with_run(command: &str, args: &[&str], run_id: &str) -> Request {
    Request {
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        run_id: run_id.to_string(),
    }
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

fn count_rows(ctx: &Context, sql: &str) -> i64 {
    let conn = store::open_readonly(ctx).unwrap().expect("db must exist");
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

/// 经真实 sources:get/set 路径替换 aliases.shell 来源（含 revision CAS）。
fn set_alias_source(ctx: &Context, value: &Value) {
    let env = execute(ctx, &request("sources:get", &["--key", "aliases.shell"]));
    assert!(env.ok, "{:?}", env.errors);
    let revision = env.data["sources"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    let wrapper = json!({ "revision": revision, "value": value }).to_string();
    let env = execute(
        ctx,
        &request(
            "sources:set",
            &["--key", "aliases.shell", "--value", &wrapper],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
}

fn expected_block(lines: &[&str]) -> String {
    format!(
        "# >>> agent-manager shell aliases >>>\n{}\n# <<< agent-manager shell aliases <<<\n",
        lines.join("\n")
    )
}

fn read(ctx: &Context, rel: &str) -> Option<Vec<u8>> {
    std::fs::read(ctx.home_dir.join(rel)).ok()
}

// 反例 1 + 8 前半：缺库时 status 不创建 DB、不写目标文件；--dry-run 已退役，
// apply 一律以 USAGE_ERROR 拒绝且零写。默认种子 4 条 alias → 两目标
// not-materialized（动作形态见私有 preflight 单测）。
#[test]
fn fresh_status_is_zero_write_not_materialized_and_dry_run_rejected() {
    let (base, ctx) = temp_ctx("fresh");

    let env = execute(&ctx, &request("aliases:status", &["--offline"]));
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.mode.offline);
    assert!(!env.lock.held);
    let elements = env.data["elements"].as_array().unwrap();
    assert_eq!(elements.len(), 2);
    assert_eq!(elements[0]["id"], "aliases:zshrc");
    assert_eq!(elements[1]["id"], "aliases:zprofile");
    assert_eq!(elements[0]["spec"]["source_key"], "aliases.shell");
    assert_eq!(elements[0]["spec"]["target"], "~/.zshrc");
    assert_eq!(elements[1]["spec"]["target"], "~/.zprofile");
    assert_eq!(
        elements[0]["spec"]["aliases"],
        json!(["chrome", "claude", "codex", "grok"])
    );
    for el in elements {
        assert_eq!(el["state"], "not-materialized");
        assert_eq!(el["observed"]["target_exists"], Value::Bool(false));
        assert_eq!(el["observed"]["block_present"], Value::Bool(false));
        assert_eq!(el["observed"]["alias_count"], 4);
        assert_eq!(el["observed"]["applied_digest"], Value::Null);
        assert_eq!(
            el["actions"],
            json!([{ "id": "sync", "risk": "write-local", "needs_lock": true, "offline_ok": true }])
        );
    }

    let env = execute(&ctx, &request("aliases:apply", &["--dry-run"]));
    assert!(!env.ok, "--dry-run 必须以参数错误拒绝: {:?}", env.data);
    assert_eq!(env.errors[0].code, "USAGE_ERROR");
    assert!(env.errors[0].message.contains("参数错误"));
    assert!(!env.lock.held, "被拒绝的 apply 不得持锁");

    assert!(
        !store::db_path(&ctx).exists(),
        "status/拒绝路径不得创建 DB（诊断日志不算 DB）"
    );
    assert!(
        !ctx.data_dir.join("agent-manager.apply.lock").exists(),
        "拒绝路径不得创建 apply lock"
    );
    assert!(!ctx.home_dir.exists(), "status/拒绝路径不得写目标文件");
    let _ = std::fs::remove_dir_all(&base);
}

// 反例 8 后半：not-materialized 真实 apply 从无到有创建区块；审计一条且无正文。
#[test]
fn apply_creates_block_from_scratch_with_audit() {
    let (base, ctx) = temp_ctx("create");
    let env = execute(
        &ctx,
        &request("aliases:apply", &["--only", "aliases:zshrc"]),
    );
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.lock.held, "real apply 必须持锁");
    assert_eq!(env.data["steps"].as_array().unwrap().len(), 1);
    assert_eq!(env.data["steps"][0]["name"], "aliases:sync aliases:zshrc");

    let content = String::from_utf8(read(&ctx, ".zshrc").unwrap()).unwrap();
    assert!(content.starts_with("# >>> agent-manager shell aliases >>>\n"));
    assert!(content.contains("alias chrome='open --env TZ=\"America/Los_Angeles\""));
    // 内置默认只带时区与语言：跳过权限确认一类的高权限参数改由用户在界面上主动勾选，
    // 不再是装完就有的默认值
    assert!(content.contains("alias claude='TZ=\"America/Los_Angeles\" LANG=\"en_US.UTF-8\" LC_ALL=\"en_US.UTF-8\" claude'"));
    assert!(!content.contains("--dangerously-skip-permissions"));
    assert!(content.contains("alias codex="));
    assert!(content.contains("alias grok="));
    assert!(content.ends_with("# <<< agent-manager shell aliases <<<\n"));
    // alias 行按名排序
    let chrome = content.find("alias chrome=").unwrap();
    let claude = content.find("alias claude=").unwrap();
    let codex = content.find("alias codex=").unwrap();
    let grok = content.find("alias grok=").unwrap();
    assert!(chrome < claude && claude < codex && codex < grok);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(ctx.home_dir.join(".zshrc"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "新文件必须为 0600");
    }

    let status = execute(&ctx, &request("aliases:status", &[]));
    assert_eq!(element(&status, "aliases:zshrc")["state"], "managed-ok");
    assert_eq!(
        count_rows(&ctx, "SELECT COUNT(*) FROM alias_target_bindings"),
        1
    );
    assert_eq!(
        count_rows(
            &ctx,
            "SELECT COUNT(*) FROM operation_logs WHERE operation_type = 'aliases:apply'"
        ),
        1
    );
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let detail: String = conn
        .query_row(
            "SELECT detail_json FROM operation_logs WHERE operation_type = 'aliases:apply'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !detail.contains("dangerously"),
        "审计 detail 不得含 alias 正文"
    );
    let _ = std::fs::remove_dir_all(&base);
}

// 反例 2：绑定后外部追加内容 → status drifted + blocked_reason，apply
// fail-closed 拒绝且文件字节不变；绑定与既有审计保留。
#[test]
fn external_append_after_apply_is_rejected_fail_closed() {
    let (base, ctx) = temp_ctx("tamper");
    let env = execute(
        &ctx,
        &request("aliases:apply", &["--only", "aliases:zshrc"]),
    );
    assert!(env.ok, "{:?}", env.errors);
    let target = ctx.home_dir.join(".zshrc");
    let mut bytes = std::fs::read(&target).unwrap();
    bytes.extend_from_slice(b"# user external edit\nexport FOO=1\n");
    std::fs::write(&target, &bytes).unwrap();

    let env = execute(&ctx, &request("aliases:status", &[]));
    let zshrc = element(&env, "aliases:zshrc");
    assert_eq!(zshrc["state"], "drifted");
    assert_eq!(
        zshrc["observed"]["blocked_reason"].as_str().unwrap(),
        "目标文件存在外部修改，拒绝覆盖"
    );
    assert_eq!(zshrc["actions"], json!([]));
    // 未被外部触碰的 zprofile 仍是 not-materialized，不受影响
    assert_eq!(
        element(&env, "aliases:zprofile")["state"],
        "not-materialized"
    );

    let audit_before = count_rows(&ctx, "SELECT COUNT(*) FROM operation_logs");
    let env = execute(
        &ctx,
        &request("aliases:apply", &["--only", "aliases:zshrc"]),
    );
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "ALIAS_ERROR");
    assert!(!env.lock.held, "preflight 即拒绝，不得取锁");
    assert_eq!(
        std::fs::read(&target).unwrap(),
        bytes,
        "外部字节必须原样保留"
    );
    assert_eq!(
        count_rows(&ctx, "SELECT COUNT(*) FROM alias_target_bindings"),
        1,
        "binding 必须保留"
    );
    assert_eq!(
        count_rows(&ctx, "SELECT COUNT(*) FROM operation_logs"),
        audit_before,
        "拒绝路径不得新增审计"
    );
    let _ = std::fs::remove_dir_all(&base);
}

// 反例 3：无绑定的 legacy 文件（前置/后置自定义 + 陈旧区块）apply 只换区块，
// 区块外逐字节保留。
#[test]
fn apply_converges_block_preserving_outer_bytes() {
    let (base, ctx) = temp_ctx("preserve");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let zshrc = ctx.home_dir.join(".zshrc");
    let pre = "# 我的自定义前置\nexport PATH=/opt/custom/bin:$PATH\n";
    let post = "# 我的自定义后置\nbindkey '^R' history-incremental-search-backward\n";
    std::fs::write(
        &zshrc,
        format!(
            "{pre}# >>> agent-manager shell aliases >>>\nalias stale='old'\n# <<< agent-manager shell aliases <<<\n{post}"
        ),
    )
    .unwrap();

    let env = execute(&ctx, &request("aliases:status", &[]));
    assert_eq!(element(&env, "aliases:zshrc")["state"], "drifted");

    let env = execute(
        &ctx,
        &request("aliases:apply", &["--only", "aliases:zshrc"]),
    );
    assert!(env.ok, "{:?}", env.errors);
    let content = String::from_utf8(std::fs::read(&zshrc).unwrap()).unwrap();
    assert!(
        content.starts_with(pre),
        "前置内容必须逐字节保留: {content}"
    );
    assert!(content.ends_with(post), "后置内容必须逐字节保留: {content}");
    assert!(!content.contains("stale"), "陈旧区块行必须被替换");
    assert!(content.contains("alias codex="));
    let status = execute(&ctx, &request("aliases:status", &[]));
    assert_eq!(element(&status, "aliases:zshrc")["state"], "managed-ok");
    let _ = std::fs::remove_dir_all(&base);
}

// 反例 4：并发第二个 apply 被全局锁拒绝。
#[test]
fn concurrent_second_apply_is_refused_by_lock() {
    let (base, ctx) = temp_ctx("lock");
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
            &request_with_run("aliases:apply", &[], "contended-aliases"),
        )
    });
    let env = worker.join().unwrap();
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "ALIAS_ERROR");
    assert!(!env.lock.held);
    assert!(!ctx.home_dir.exists(), "锁拒绝后不得写任何目标文件");
    drop(lock_file);

    // 释放锁后 apply 正常执行
    let env = execute(&ctx, &request("aliases:apply", &[]));
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["steps"].as_array().unwrap().len(), 2);
    let _ = std::fs::remove_dir_all(&base);
}

// 反例 5：--only aliases:zshrc 不影响 zprofile。
#[test]
fn only_zshrc_leaves_zprofile_untouched() {
    let (base, ctx) = temp_ctx("only");
    let env = execute(
        &ctx,
        &request("aliases:apply", &["--only", "aliases:zshrc"]),
    );
    assert!(env.ok, "{:?}", env.errors);
    assert!(ctx.home_dir.join(".zshrc").exists());
    assert!(
        !ctx.home_dir.join(".zprofile").exists(),
        "zprofile 不得被创建"
    );
    assert_eq!(
        count_rows(
            &ctx,
            "SELECT COUNT(*) FROM alias_target_bindings WHERE target_id = 'aliases:zprofile'"
        ),
        0,
        "zprofile 不得登记绑定"
    );
    let status = execute(&ctx, &request("aliases:status", &[]));
    assert_eq!(element(&status, "aliases:zshrc")["state"], "managed-ok");
    assert_eq!(
        element(&status, "aliases:zprofile")["state"],
        "not-materialized"
    );
    let _ = std::fs::remove_dir_all(&base);
}

// 反例 6：非法 alias 名（空格/引号注入）与疑似凭据值被 sources:set 拒绝；
// revision 过期拒绝；--dry-run 已退役，携带即拒绝且对库零写。
#[test]
fn sources_set_rejects_injection_and_secret_values() {
    let (base, ctx) = temp_ctx("source-reject");
    let env = execute(&ctx, &request("sources:get", &["--key", "aliases"]));
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["sources"].as_array().unwrap().len(), 1);
    assert_eq!(env.data["sources"][0]["key"], "aliases.shell");
    assert_eq!(env.data["sources"][0]["format"], "json");
    let revision = env.data["sources"][0]["revision"]
        .as_str()
        .unwrap()
        .to_string();

    let try_set = |value: Value| -> agent_manager_engine::Envelope {
        let wrapper = json!({ "revision": revision, "value": value }).to_string();
        execute(
            &ctx,
            &request(
                "sources:set",
                &["--key", "aliases.shell", "--value", &wrapper],
            ),
        )
    };

    for bad in [
        json!({"aliases": {"bad name": "echo x"}}),
        json!({"aliases": {"bad'name": "echo x"}}),
        json!({"aliases": {"ok": "inject'ed"}}),
        json!({"aliases": {"ok": "line1\nline2"}}),
        json!({"aliases": {"ok": 42}}),
        json!({"unknown": {}}),
    ] {
        let env = try_set(bad);
        assert!(!env.ok, "{:?} 应被拒绝", env.data);
        assert_eq!(env.errors[0].code, "SOURCE_ERROR");
    }
    let secret = json!({"aliases": {"ok": "curl -H \"Authorization: Bearer sk-abcdefghijklmnopqrstuvwxyz123456\" example.com"}});
    let env = try_set(secret);
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "SOURCE_ERROR");
    assert!(env.errors[0].message.contains("疑似凭据"));
    assert!(
        !store::db_path(&ctx).exists(),
        "所有拒绝路径不得创建 DB（含预检后的校验失败；诊断日志不算 DB）"
    );

    // 合法保存：先真实 set 建库；--dry-run 已退役，携带即拒绝且对库零写
    let good = json!({"timezone": "UTC", "lang": "C", "aliases": {"ll": "ls -l"}});
    set_alias_source(&ctx, &good);
    let db_bytes = std::fs::read(store::db_path(&ctx)).expect("真实 sources:set 必须建库");
    let wrapper = json!({ "revision": revision, "value": good }).to_string();
    let env = execute(
        &ctx,
        &request(
            "sources:set",
            &["--key", "aliases.shell", "--value", &wrapper, "--dry-run"],
        ),
    );
    assert!(!env.ok, "--dry-run 必须拒绝: {:?}", env.data);
    assert_eq!(env.errors[0].code, "SOURCE_ERROR");
    assert!(!env.lock.held, "被拒绝的 sources:set 不得持锁");
    assert_eq!(
        std::fs::read(store::db_path(&ctx)).unwrap(),
        db_bytes,
        "被拒绝的 --dry-run 不得改写 DB"
    );
    assert!(!ctx.home_dir.exists(), "sources:set 拒绝路径不得写 home");

    // 真实保存已使旧 revision 过期：复用同一库重放即拒绝，不重复初始化
    let env = execute(
        &ctx,
        &request(
            "sources:set",
            &[
                "--key",
                "aliases.shell",
                "--value",
                &json!({ "revision": revision, "value": good }).to_string(),
            ],
        ),
    );
    assert!(!env.ok, "过期 revision 必须拒绝");
    assert!(env.errors[0].message.contains("revision"));
    let env = execute(&ctx, &request("aliases:status", &[]));
    let el = element(&env, "aliases:zshrc");
    assert_eq!(el["spec"]["aliases"], json!(["ll"]));
    let env = execute(
        &ctx,
        &request("aliases:apply", &["--only", "aliases:zshrc"]),
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        String::from_utf8(read(&ctx, ".zshrc").unwrap()).unwrap(),
        expected_block(&["alias ll='ls -l'"])
    );
    let _ = std::fs::remove_dir_all(&base);
}

// 反例 7：重复 apply 幂等 no-op；--dry-run 已退役，apply 携带即拒绝且零写。
#[test]
fn repeat_apply_is_noop_and_dry_run_is_rejected() {
    let (base, ctx) = temp_ctx("idempotent");
    let dry = execute(&ctx, &request("aliases:apply", &["--dry-run"]));
    assert!(!dry.ok, "--dry-run 必须以参数错误拒绝");
    assert_eq!(dry.errors[0].code, "USAGE_ERROR");
    assert!(!dry.lock.held);
    assert!(!ctx.home_dir.exists());

    let first = execute(&ctx, &request("aliases:apply", &[]));
    assert!(first.ok, "{:?}", first.errors);
    assert_eq!(first.data["steps"].as_array().unwrap().len(), 2);
    let zshrc_bytes = read(&ctx, ".zshrc").unwrap();
    let zprofile_bytes = read(&ctx, ".zprofile").unwrap();

    let second = execute(&ctx, &request("aliases:apply", &[]));
    assert!(second.ok, "{:?}", second.errors);
    assert_eq!(
        second.data["steps"].as_array().unwrap().len(),
        0,
        "重复 apply 必须是 no-op"
    );
    assert_eq!(read(&ctx, ".zshrc").unwrap(), zshrc_bytes);
    assert_eq!(read(&ctx, ".zprofile").unwrap(), zprofile_bytes);
    assert_eq!(
        count_rows(&ctx, "SELECT COUNT(*) FROM alias_target_bindings"),
        2
    );
    assert_eq!(
        count_rows(
            &ctx,
            "SELECT COUNT(*) FROM operation_logs WHERE operation_type = 'aliases:apply'"
        ),
        1,
        "no-op 不得新增审计"
    );

    let status = execute(&ctx, &request("aliases:status", &[]));
    for id in ["aliases:zshrc", "aliases:zprofile"] {
        assert_eq!(element(&status, id)["state"], "managed-ok");
        assert_eq!(element(&status, id)["actions"], json!([]));
    }
    let _ = std::fs::remove_dir_all(&base);
}

// 参数反例：非法 --only / 位置参数 / 未知选项 / 重复 flag 一律 USAGE_ERROR
// 且零写、不回显取值。
#[test]
fn invalid_args_return_usage_error_without_echoing_values() {
    let (base, ctx) = temp_ctx("invalid-args");
    let marker = "ALIASES-ARG-MARKER-c391";
    for (command, args) in [
        ("aliases:status", vec!["--only", "aliases:zshrc"]),
        ("aliases:status", vec!["--dry-run"]),
        ("aliases:apply", vec!["--only", marker]),
        (
            "aliases:apply",
            vec!["--only", "aliases:zshrc", "--only", "aliases:zshrc"],
        ),
        ("aliases:apply", vec!["--dry-run"]),
        ("aliases:apply", vec!["--bogus"]),
        ("aliases:apply", vec!["positional"]),
        ("aliases:apply", vec!["--dry-run", "--dry-run"]),
    ] {
        let env = execute(&ctx, &request_with_run(command, &args, "aliases-invalid"));
        assert!(!env.ok, "{command} {args:?} 应失败");
        assert_eq!(
            env.errors[0].code, "USAGE_ERROR",
            "{command} {args:?} 必须映射 USAGE_ERROR"
        );
        assert!(!serde_json::to_string(&env).unwrap().contains(marker));
        if let Some(log_file) = &env.log_file {
            assert!(!std::fs::read_to_string(log_file).unwrap().contains(marker));
        }
    }
    assert!(!ctx.data_dir.join("agent-manager.apply.lock").exists());
    assert!(!ctx.home_dir.exists());
    let _ = std::fs::remove_dir_all(&base);
}

// 第二目标写绑定失败：两文件与 DB 事务整体回滚。
#[test]
fn second_target_binding_failure_rolls_back_files_and_db() {
    let (base, ctx) = temp_ctx("rollback-late");
    {
        let conn = store::open_write(&ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE alias_target_bindings (
                target_id TEXT PRIMARY KEY NOT NULL,
                applied_digest TEXT NOT NULL,
                source_revision TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TRIGGER fail_zprofile_binding BEFORE INSERT ON alias_target_bindings
             WHEN NEW.target_id = 'aliases:zprofile'
             BEGIN SELECT RAISE(ABORT, 'injected-zprofile-binding-failure'); END;",
        )
        .unwrap();
    }
    let env = execute(&ctx, &request("aliases:apply", &[]));
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "ALIAS_ERROR");
    assert!(env.lock.held, "已进入真实 apply，envelope lock.held=true");
    assert!(
        env.errors[0]
            .message
            .contains("injected-zprofile-binding-failure"),
        "错误须包含注入失败标记: {}",
        env.errors[0].message
    );
    assert!(
        !ctx.home_dir.join(".zshrc").exists(),
        "第一目标已写文件必须回滚删除"
    );
    assert!(!ctx.home_dir.join(".zprofile").exists(), "第二目标不得残留");
    assert_eq!(
        count_rows(&ctx, "SELECT COUNT(*) FROM alias_target_bindings"),
        0,
        "DB 事务必须整体回滚"
    );
    assert_eq!(
        count_rows(
            &ctx,
            "SELECT COUNT(*) FROM operation_logs WHERE operation_type='aliases:apply'"
        ),
        0
    );
    let _ = std::fs::remove_dir_all(&base);
}

// 审计写入失败：已写文件与绑定回滚。
#[test]
fn audit_failure_rolls_back_files_and_bindings() {
    let (base, ctx) = temp_ctx("rollback-audit");
    {
        let conn = store::open_write(&ctx).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_operation_logs BEFORE INSERT ON operation_logs
             BEGIN SELECT RAISE(ABORT, 'injected-end-failure'); END;",
        )
        .unwrap();
    }
    let env = execute(
        &ctx,
        &request("aliases:apply", &["--only", "aliases:zshrc"]),
    );
    assert!(!env.ok);
    assert!(
        !ctx.home_dir.join(".zshrc").exists(),
        "审计失败后已写文件必须回滚"
    );
    assert_eq!(
        count_rows(&ctx, "SELECT COUNT(*) FROM alias_target_bindings"),
        0
    );
    let _ = std::fs::remove_dir_all(&base);
}

// 来源行损坏（结构非法/非 canonical/revision 不符/未知 key/坏 schema）一律
// fail-closed 且不回显正文 marker。
#[test]
fn corrupt_alias_source_rows_fail_closed_without_leaking_markers() {
    let source_marker = "ALIAS-SOURCE-MARKER-77aa";
    let sha = agent_manager_engine::native_mcp::model::sha256_hex;

    let seed = |ctx: &Context, value_json: String, revision: String| {
        let conn = store::open_write(ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE alias_sources (
                key TEXT PRIMARY KEY NOT NULL,
                value_json TEXT NOT NULL,
                revision TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO alias_sources VALUES ('aliases.shell', ?1, ?2, '2026-01-01')",
            rusqlite::params![value_json, revision],
        )
        .unwrap();
    };

    let assert_fail_closed = |ctx: &Context| {
        for command in ["sources:get", "aliases:status", "aliases:apply"] {
            let args: &[&str] = if command == "sources:get" {
                &["--key", "aliases.shell"]
            } else {
                &[]
            };
            let env = execute(
                ctx,
                &request_with_run(command, args, "corrupt-alias-source"),
            );
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

    // 1) 结构非法（alias 值含单引号）且含 marker
    {
        let (base, ctx) = temp_ctx("corrupt-alias-structure");
        let bad = json!({"aliases": {"ok": format!("bad'{source_marker}'")}}).to_string();
        let rev = sha(&bad);
        seed(&ctx, bad, rev);
        assert_fail_closed(&ctx);
        assert!(!ctx.home_dir.exists());
        let _ = std::fs::remove_dir_all(&base);
    }
    // 2) value_json 非合法 JSON
    {
        let (base, ctx) = temp_ctx("corrupt-alias-json");
        seed(&ctx, format!("{{not-json-{source_marker}"), sha("x"));
        let env = execute(&ctx, &request("sources:get", &["--key", "aliases.shell"]));
        assert!(!env.ok);
        assert_eq!(env.errors[0].code, "SOURCE_ERROR");
        assert!(!serde_json::to_string(&env).unwrap().contains(source_marker));
        let env = execute(&ctx, &request("aliases:status", &[]));
        assert!(!env.ok);
        assert_eq!(env.errors[0].code, "ALIAS_ERROR");
        let _ = std::fs::remove_dir_all(&base);
    }
    // 3) revision 与 value_json 不一致
    {
        let (base, ctx) = temp_ctx("corrupt-alias-revision");
        let good = json!({"aliases": {}}).to_string();
        seed(&ctx, good, sha("other"));
        let env = execute(&ctx, &request("aliases:status", &[]));
        assert!(!env.ok);
        let _ = std::fs::remove_dir_all(&base);
    }
    // 4) 未知 key 行
    {
        let (base, ctx) = temp_ctx("corrupt-alias-key");
        let conn = store::open_write(&ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE alias_sources (
                key TEXT PRIMARY KEY NOT NULL,
                value_json TEXT NOT NULL,
                revision TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .unwrap();
        let value = json!({"aliases": {}}).to_string();
        conn.execute(
            "INSERT INTO alias_sources VALUES (?1, ?2, ?3, '2026-01-01')",
            rusqlite::params![format!("aliases.unknown-{source_marker}"), value, sha("x")],
        )
        .unwrap();
        drop(conn);
        let env = execute(&ctx, &request("aliases:status", &[]));
        assert!(!env.ok);
        assert!(!serde_json::to_string(&env).unwrap().contains(source_marker));
        let _ = std::fs::remove_dir_all(&base);
    }
    // 5) 坏 schema（3 列）
    {
        let (base, ctx) = temp_ctx("corrupt-alias-schema");
        let conn = store::open_write(&ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE alias_sources (
                key TEXT PRIMARY KEY NOT NULL,
                value_json TEXT NOT NULL,
                revision TEXT NOT NULL
            )",
        )
        .unwrap();
        drop(conn);
        let env = execute(&ctx, &request("aliases:status", &[]));
        assert!(!env.ok);
        assert_eq!(env.errors[0].code, "ALIAS_ERROR");
        let _ = std::fs::remove_dir_all(&base);
    }
}

// 不安全目标（符号链接）在 status 降级为 drifted + blocked_reason，
// apply 拒绝，链接目标字节不变。
#[cfg(unix)]
#[test]
fn symlink_target_is_blocked_without_writes() {
    let (base, ctx) = temp_ctx("symlink");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let elsewhere = base.join("elsewhere-zshrc");
    std::fs::write(&elsewhere, b"# elsewhere\n").unwrap();
    std::os::unix::fs::symlink(&elsewhere, ctx.home_dir.join(".zshrc")).unwrap();

    let env = execute(&ctx, &request("aliases:status", &[]));
    let zshrc = element(&env, "aliases:zshrc");
    assert_eq!(zshrc["state"], "drifted");
    assert!(zshrc["observed"]["blocked_reason"].as_str().is_some());
    assert_eq!(zshrc["actions"], json!([]));

    let env = execute(
        &ctx,
        &request("aliases:apply", &["--only", "aliases:zshrc"]),
    );
    assert!(!env.ok);
    assert!(!env.lock.held, "preflight 拒绝不得持锁");
    assert_eq!(std::fs::read(&elsewhere).unwrap(), b"# elsewhere\n");
    // 拒绝发生在 preflight，可能未建库；若已建库则不得有绑定行。
    if store::db_path(&ctx).exists() {
        assert_eq!(
            count_rows(&ctx, "SELECT COUNT(*) FROM alias_target_bindings"),
            0
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

// 空 alias 来源：目标缺失视为 managed-ok 且不建文件；已有区块被移除、
// 外部内容保留。
#[test]
fn empty_aliases_remove_block_and_create_nothing() {
    let (base, ctx) = temp_ctx("empty");
    set_alias_source(&ctx, &json!({"aliases": {}}));

    let env = execute(&ctx, &request("aliases:status", &[]));
    for id in ["aliases:zshrc", "aliases:zprofile"] {
        assert_eq!(element(&env, id)["state"], "managed-ok");
        assert_eq!(element(&env, id)["actions"], json!([]));
    }
    let env = execute(&ctx, &request("aliases:apply", &[]));
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["steps"].as_array().unwrap().len(), 0);
    assert!(!ctx.home_dir.exists(), "空 alias 不得创建目标文件");

    // 已有区块（无绑定 legacy 文件）→ drifted，apply 移除区块保留外部
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let zshrc = ctx.home_dir.join(".zshrc");
    std::fs::write(
        &zshrc,
        "pre\n# >>> agent-manager shell aliases >>>\nalias old='1'\n# <<< agent-manager shell aliases <<<\npost\n",
    )
    .unwrap();
    let env = execute(&ctx, &request("aliases:status", &[]));
    assert_eq!(element(&env, "aliases:zshrc")["state"], "drifted");
    let env = execute(
        &ctx,
        &request("aliases:apply", &["--only", "aliases:zshrc"]),
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        String::from_utf8(std::fs::read(&zshrc).unwrap()).unwrap(),
        "pre\npost\n"
    );
    let status = execute(&ctx, &request("aliases:status", &[]));
    assert_eq!(element(&status, "aliases:zshrc")["state"], "managed-ok");
    let _ = std::fs::remove_dir_all(&base);
}

// 来源变更 → drifted，apply 收敛并更新绑定；status 暴露区块级 expected/
// actual_block_digest（动作 before/after 区块 sha256 见私有 preflight 单测）。
#[test]
fn source_change_drifts_and_converges_with_block_digests() {
    let (base, ctx) = temp_ctx("source-drift");
    set_alias_source(&ctx, &json!({"aliases": {"ll": "ls -l"}}));
    let env = execute(
        &ctx,
        &request("aliases:apply", &["--only", "aliases:zshrc"]),
    );
    assert!(env.ok, "{:?}", env.errors);

    set_alias_source(&ctx, &json!({"aliases": {"ll": "ls -alh"}}));
    let env = execute(&ctx, &request("aliases:status", &[]));
    let zshrc = element(&env, "aliases:zshrc");
    assert_eq!(zshrc["state"], "drifted");
    assert_eq!(zshrc["observed"]["blocked_reason"], Value::Null);
    assert_ne!(
        zshrc["observed"]["expected_digest"],
        zshrc["observed"]["actual_block_digest"]
    );
    let block_sha = |text: &str| agent_manager_engine::native_mcp::model::sha256_hex(text);
    assert_eq!(
        zshrc["observed"]["actual_block_digest"].as_str().unwrap(),
        block_sha("alias ll='ls -l'")
    );
    assert_eq!(
        zshrc["observed"]["expected_digest"].as_str().unwrap(),
        block_sha("alias ll='ls -alh'")
    );

    let env = execute(
        &ctx,
        &request("aliases:apply", &["--only", "aliases:zshrc"]),
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        String::from_utf8(read(&ctx, ".zshrc").unwrap()).unwrap(),
        expected_block(&["alias ll='ls -alh'"])
    );
    let status = execute(&ctx, &request("aliases:status", &[]));
    assert_eq!(element(&status, "aliases:zshrc")["state"], "managed-ok");
    let _ = std::fs::remove_dir_all(&base);
}

// 保存定义不执行动作：sources:set 后目标文件/绑定不变。
#[test]
fn saving_source_never_applies() {
    let (base, ctx) = temp_ctx("no-auto-apply");
    set_alias_source(&ctx, &json!({"aliases": {"ll": "ls -l"}}));
    assert!(!ctx.home_dir.exists(), "保存定义不得写目标文件");
    assert_eq!(
        count_rows(&ctx, "SELECT COUNT(*) FROM alias_target_bindings"),
        0
    );
    let status = execute(&ctx, &request("aliases:status", &[]));
    assert_eq!(
        element(&status, "aliases:zshrc")["state"],
        "not-materialized"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// 逐条生效回读：配置文件里现在到底是哪一条跟上了、哪一条没跟上。
#[test]
fn status_reports_each_entry_against_the_shell_file() {
    let (root, ctx) = temp_ctx("entry-readback");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();

    // 区块里：claude 与期望一致、codex 是旧取值、grok 与 chrome 缺失
    let block = "# >>> agent-manager shell aliases >>>\n\
alias chrome='open --env TZ=\"America/Los_Angeles\" --env LANG=\"en_US.UTF-8\" --env LC_ALL=\"en_US.UTF-8\" -n -a \"Google Chrome\" --args --lang=en-US'\n\
alias claude='TZ=\"America/Los_Angeles\" LANG=\"en_US.UTF-8\" LC_ALL=\"en_US.UTF-8\" claude'\n\
alias codex='old-value'\n\
# <<< agent-manager shell aliases <<<\n";
    std::fs::write(ctx.home_dir.join(".zshrc"), block).unwrap();

    let env = execute(&ctx, &request("aliases:status", &[]));
    assert!(env.ok, "{:?}", env.errors);
    let zshrc = env.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|el| el["id"] == "aliases:zshrc")
        .unwrap();
    let entries = zshrc["observed"]["entries"].as_array().unwrap();
    let find = |name: &str| {
        entries
            .iter()
            .find(|e| e["name"] == name)
            .unwrap_or_else(|| panic!("缺少条目 {name}"))
    };
    assert_eq!(find("claude")["state"], "applied", "与期望一致的报已生效");
    assert_eq!(find("codex")["state"], "outdated", "取值不同的报待同步");
    assert_eq!(find("grok")["state"], "missing", "区块里没有的报缺失");
    // 实际取值如实回读，便于界面直接展示「现在是什么」
    assert_eq!(find("codex")["actual"], "alias codex='old-value'");
    assert!(find("grok")["actual"].is_null());
    // 内置默认不再预置跳过权限确认的参数
    assert_eq!(
        find("claude")["expected"],
        "alias claude='TZ=\"America/Los_Angeles\" LANG=\"en_US.UTF-8\" LC_ALL=\"en_US.UTF-8\" claude'"
    );

    // 目标文件还不存在时，逐条都报「尚未写入」而不是「缺失」
    let (root2, ctx2) = temp_ctx("entry-readback-fresh");
    std::fs::create_dir_all(&ctx2.home_dir).unwrap();
    let fresh = execute(&ctx2, &request("aliases:status", &[]));
    let fresh_entries = fresh.data["elements"].as_array().unwrap()[0]["observed"]["entries"]
        .as_array()
        .unwrap()
        .clone();
    assert!(fresh_entries
        .iter()
        .all(|e| e["state"] == "not-materialized"));

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(root2);
}
