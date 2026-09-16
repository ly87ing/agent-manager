use chrono::{Duration, Utc};
use serde_json::Value;
use std::path::PathBuf;

use agent_manager_engine::native_logs::{
    is_app_log_file, parse_log_file_date, prune_expired_jsonl_logs, sanitize_diagnostic_message,
    startup_maintenance,
};
use agent_manager_engine::{execute, store, Context, Request};

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    let base = std::env::temp_dir().join(format!("am-logs-test-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&base);
    let ctx = Context {
        data_dir: base.join("data"),
        home_dir: base.join("home"),
    };
    (base, ctx)
}

fn req(command: &str, args: &[&str]) -> Request {
    Request {
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        run_id: "test-run-logs-1".to_string(),
    }
}

#[test]
fn readonly_commands_never_create_logs_directory() {
    let (base, ctx) = temp_ctx("readonly-zero-write");
    let logs_dir = ctx.data_dir.join("logs");

    // 各类只读请求
    let env_status = execute(&ctx, &req("agents:status", &[]));
    assert!(env_status.ok);
    assert!(env_status.log_file.is_none());
    assert!(!logs_dir.exists(), "agents:status 不应创建 logs 目录");

    let env_policy = execute(&ctx, &req("policy:get", &[]));
    assert!(env_policy.ok);
    assert!(env_policy.log_file.is_none());
    assert!(!logs_dir.exists(), "policy:get 不应创建 logs 目录");

    let env_runs = execute(&ctx, &req("runs:list", &[]));
    assert!(env_runs.ok);
    assert!(env_runs.log_file.is_none());
    assert!(!logs_dir.exists(), "runs:list 不应创建 logs 目录");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn rejected_dry_run_mutating_command_is_logged_without_creating_store() {
    let (base, ctx) = temp_ctx("dry-run-logged");
    let logs_dir = ctx.data_dir.join("logs");

    // dry-run 已退役且不再享有日志豁免：被拒绝的写动作按失败写请求统一记录
    // 诊断日志；日志不是目标状态——SQLite 库仍不得创建。
    let env_dry = execute(
        &ctx,
        &req(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.codex.enabled",
                "--value",
                "true",
                "--dry-run",
            ],
        ),
    );
    assert!(
        !env_dry.ok,
        "--dry-run 已退役，必须拒绝: {:?}",
        env_dry.data
    );
    assert_eq!(env_dry.errors[0].code, "USAGE_ERROR");
    assert!(env_dry.log_file.is_some(), "失败写动作必须记录诊断日志");
    let log_path = PathBuf::from(env_dry.log_file.as_ref().unwrap());
    assert!(log_path.exists());
    assert!(log_path.starts_with(&logs_dir));
    let content = std::fs::read_to_string(&log_path).unwrap();
    let parsed: Value = serde_json::from_str(content.trim()).unwrap();
    assert_eq!(parsed["command"], "policy:set");
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["error_code"], "USAGE_ERROR");
    assert!(
        !store::db_path(&ctx).exists(),
        "诊断日志不属于目标状态：不得创建 SQLite 库"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn mutating_command_records_structured_jsonl_log_and_unix_permissions() {
    let (base, ctx) = temp_ctx("mutating-success");
    let logs_dir = ctx.data_dir.join("logs");

    let env = execute(
        &ctx,
        &req(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "custom.log_test",
                "--value",
                "\"hello\"",
            ],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.log_file.is_some());
    let log_file_str = env.log_file.unwrap();
    let log_path = PathBuf::from(&log_file_str);
    assert!(log_path.exists(), "返回的日志文件路径应真实存在");
    assert!(log_path.starts_with(&logs_dir));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir_mode = std::fs::metadata(&logs_dir).unwrap().permissions().mode();
        assert_eq!(dir_mode & 0o777, 0o700, "logs 目录权限应为 0700");

        let file_mode = std::fs::metadata(&log_path).unwrap().permissions().mode();
        assert_eq!(file_mode & 0o777, 0o600, "日志文件权限应为 0600");
    }

    // 验证日志行结构
    let content = std::fs::read_to_string(&log_path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 1);

    let parsed: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(parsed["command"], "policy:set");
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["run_id"], "test-run-logs-1");
    assert!(parsed["timestamp"].is_string());
    assert!(parsed["duration_ms"].is_number());
    assert!(parsed.get("args").is_none(), "严禁记录 args");
    assert!(parsed.get("payload").is_none(), "严禁记录 payload");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn mutating_failure_records_diagnostics_and_sanitizes_content() {
    let (base, ctx) = temp_ctx("mutating-failure");
    std::fs::create_dir_all(&ctx.data_dir).unwrap();

    // 触发非法 key 错误，并在参数中混入假敏感信息
    let env = execute(
        &ctx,
        &req(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "invalid..key",
                "--value",
                "true",
            ],
        ),
    );
    assert!(!env.ok);
    assert!(env.log_file.is_some());
    let log_path = PathBuf::from(env.log_file.as_ref().unwrap());
    assert!(log_path.exists());

    let content = std::fs::read_to_string(&log_path).unwrap();
    let parsed: Value = serde_json::from_str(content.trim()).unwrap();
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["error_code"], "USAGE_ERROR");
    assert!(parsed["error_message"]
        .as_str()
        .unwrap()
        .contains("非法 key"));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn centralized_redaction_masks_credentials_and_preserves_error_chain() {
    // 1. 私钥内容屏蔽
    let privkey = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA0abc123...\n-----END RSA PRIVATE KEY-----";
    let masked_key = sanitize_diagnostic_message(&format!("Key loading failed: {privkey}"));
    assert!(!masked_key.contains("MIIEowIBAAKCAQEA0abc123"));
    assert!(masked_key.contains("[REDACTED PRIVATE KEY]"));

    // 2. Bearer 令牌屏蔽
    let bearer_str = "Authorization: Bearer my-super-secret-token-xyz123";
    let masked_bearer = sanitize_diagnostic_message(bearer_str);
    assert!(!masked_bearer.contains("my-super-secret-token-xyz123"));
    assert!(masked_bearer.contains("Bearer ***"));

    // 3. URL user:pass 屏蔽
    let url_str = "Failed to clone https://bot_user:secret_pwd_999@gitlab.example.com/repo.git";
    let masked_url = sanitize_diagnostic_message(url_str);
    assert!(!masked_url.contains("secret_pwd_999"));
    assert!(masked_url.contains("https://bot_user:***@gitlab.example.com/repo.git"));

    // 4. URL query 参数屏蔽
    let query_str = "Request failed: https://api.service.com/v1/sync?token=tok_secret_888&api_key=ak_secret_777&debug=true";
    let masked_query = sanitize_diagnostic_message(query_str);
    assert!(!masked_query.contains("tok_secret_888"));
    assert!(!masked_query.contains("ak_secret_777"));
    assert!(masked_query.contains("token=***"));
    assert!(masked_query.contains("api_key=***"));
    assert!(masked_query.contains("debug=true"));

    // 5. 常见键值对屏蔽 (双引号带空格、单引号带空格、无引号值)
    let kv_str = "Config invalid: password=\"top secret value with spaces\", token='single quoted secret phrase', api_key: bare_secret_val_123, other=\"normal value\"";
    let masked_kv = sanitize_diagnostic_message(kv_str);
    assert!(!masked_kv.contains("top secret value with spaces"));
    assert!(!masked_kv.contains("secret value"));
    assert!(masked_kv.contains("password=\"***\""));
    assert!(!masked_kv.contains("single quoted secret phrase"));
    assert!(!masked_kv.contains("secret phrase"));
    assert!(masked_kv.contains("token='***'"));
    assert!(!masked_kv.contains("bare_secret_val_123"));
    assert!(masked_kv.contains("api_key: ***"));
    assert!(masked_kv.contains("other=\"normal value\""));

    // 6. 知名 token 模式 (GitHub / OpenAI)
    let tokens_str =
        "Found tokens ghp_1234567890abcdef1234567890abcdef1234 and sk-123456789012345678901234";
    let masked_tokens = sanitize_diagnostic_message(tokens_str);
    assert!(!masked_tokens.contains("1234567890abcdef"));
    assert!(masked_tokens.contains("gh***"));
    assert!(masked_tokens.contains("sk-***"));

    // 7. anyhow 错误链保留测试
    let root_cause = anyhow::anyhow!("HTTP 401: Bearer token_leak_123 rejected");
    let wrap_err = anyhow::anyhow!("upstream sync failed: {root_cause:#}");
    let formatted = format!("{wrap_err:#}");
    let sanitized_chain = sanitize_diagnostic_message(&formatted);
    assert!(sanitized_chain.starts_with("upstream sync failed: HTTP 401: Bearer *** rejected"));
    assert!(!sanitized_chain.contains("token_leak_123"));
}

#[test]
fn log_retention_prunes_expired_files_and_preserves_boundaries() {
    let (base, ctx) = temp_ctx("retention-boundaries");
    let logs_dir = ctx.data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir).unwrap();

    let today = Utc::now().date_naive();
    let fmt_date = |days_ago: i64| {
        let d = today - Duration::days(days_ago);
        d.format("%Y-%m-%d").to_string()
    };

    // 10 天保留期
    let retention_days = 10;
    // 构造测试文件：
    // - 15 天前：已过期（应删除）
    // - 11 天前：已过期（应删除）
    // - 10 天前：刚好在阈值边界上（应保留）
    // - 9 天前：在阈值内（应保留）
    // - 今天：在阈值内（应保留）
    let f_15 = logs_dir.join(format!("agent-manager-{}.jsonl", fmt_date(15)));
    let f_11 = logs_dir.join(format!("agent-manager-{}.jsonl", fmt_date(11)));
    let f_10 = logs_dir.join(format!("agent-manager-{}.jsonl", fmt_date(10)));
    let f_9 = logs_dir.join(format!("agent-manager-{}.jsonl", fmt_date(9)));
    let f_today = logs_dir.join(format!("agent-manager-{}.jsonl", fmt_date(0)));

    for f in [&f_15, &f_11, &f_10, &f_9, &f_today] {
        std::fs::write(f, "{}\n").unwrap();
    }

    let pruned = prune_expired_jsonl_logs(&logs_dir, retention_days).unwrap();
    assert_eq!(pruned, 2, "应清理 15 天与 11 天前共 2 个过期文件");

    assert!(!f_15.exists(), "15 天前的文件应被删除");
    assert!(!f_11.exists(), "11 天前的文件应被删除");
    assert!(f_10.exists(), "10 天前的文件（边界）应保留");
    assert!(f_9.exists(), "9 天前的文件应保留");
    assert!(f_today.exists(), "今天的文件应保留");

    // 0 禁用自动清理测试
    let f_old = logs_dir.join(format!("agent-manager-{}.jsonl", fmt_date(100)));
    std::fs::write(&f_old, "{}\n").unwrap();
    let pruned_zero = prune_expired_jsonl_logs(&logs_dir, 0).unwrap();
    assert_eq!(pruned_zero, 0);
    assert!(f_old.exists(), "retention_days = 0 时不得删除任何过期文件");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn symlinks_and_non_app_files_are_protected_from_pruning_and_clearing() {
    let (base, ctx) = temp_ctx("symlink-protection");
    let logs_dir = ctx.data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir).unwrap();

    let external_target = base.join("important-external.txt");
    std::fs::write(&external_target, "critical data").unwrap();

    let today = Utc::now().date_naive();
    let old_date = (today - Duration::days(30)).format("%Y-%m-%d").to_string();

    // 在 logs 目录下创建软连接（文件名伪装为过期日志）
    let symlink_path = logs_dir.join(format!("agent-manager-{old_date}.jsonl"));
    #[cfg(unix)]
    std::os::unix::fs::symlink(&external_target, &symlink_path).unwrap();

    // 非本应用文件与格式不匹配文件
    let other_file = logs_dir.join("other-service.log");
    std::fs::write(&other_file, "other logs").unwrap();
    let wrong_format = logs_dir.join("agent-manager-invaliddate.jsonl");
    std::fs::write(&wrong_format, "invalid date format").unwrap();

    // 正常过期文件
    let real_expired = logs_dir.join(format!("agent-manager-{old_date}-real.jsonl")); // 非严格命名
    std::fs::write(&real_expired, "not strictly formatted").unwrap();

    let real_valid_expired = logs_dir.join(format!(
        "agent-manager-{}.jsonl",
        (today - Duration::days(40)).format("%Y-%m-%d")
    ));
    std::fs::write(&real_valid_expired, "valid log").unwrap();

    // 1. 过期清理阶段
    let pruned = prune_expired_jsonl_logs(&logs_dir, 10).unwrap();
    assert_eq!(pruned, 1, "只应清理 real_valid_expired 文件");
    assert!(!real_valid_expired.exists());
    assert!(external_target.exists(), "外部软链接目标绝不可被删除");
    #[cfg(unix)]
    assert!(
        symlink_path.symlink_metadata().is_ok(),
        "软链接本身不可被删除"
    );
    assert!(other_file.exists(), "非本应用文件必须保留");
    assert!(wrong_format.exists(), "非规范格式文件必须保留");

    // 2. logs:clear 阶段：清空全部本应用日志
    let env_clear = execute(&ctx, &req("logs:clear", &[]));
    assert!(env_clear.ok);
    // 软链接、其他文件、不匹配文件都不属于普通有效日志，不被删除
    assert!(external_target.exists(), "logs:clear 绝不能动外部文件");
    #[cfg(unix)]
    assert!(
        symlink_path.symlink_metadata().is_ok(),
        "logs:clear 拒绝操作软链接"
    );
    assert!(other_file.exists());
    assert!(wrong_format.exists());

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn logs_clear_zero_writes_on_absent_dir_and_preserves_sqlite_runs() {
    let (base_empty, ctx_empty) = temp_ctx("clear-empty");
    let logs_dir_empty = ctx_empty.data_dir.join("logs");

    // 缺目录零写入
    let env_empty = execute(&ctx_empty, &req("logs:clear", &[]));
    assert!(env_empty.ok);
    assert_eq!(env_empty.data["deleted"], 0);
    assert_eq!(env_empty.data["domain"], "logs");
    assert!(!logs_dir_empty.exists(), "缺目录时不应创建 logs 目录");
    let _ = std::fs::remove_dir_all(&base_empty);

    let (base, ctx) = temp_ctx("clear-populated");
    let logs_dir = ctx.data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir).unwrap();

    // 创建 3 个合法的日志文件
    for i in 1..=3 {
        let f = logs_dir.join(format!("agent-manager-2026-09-0{i}.jsonl"));
        std::fs::write(&f, "line\n").unwrap();
    }

    // 在 SQLite 中写入一条 operation_log 记录
    let conn = store::open_write(&ctx).unwrap();
    store::record_operation(&conn, Some("run-test"), "policy:set", None, None).unwrap();
    drop(conn);

    let runs_before = execute(&ctx, &req("runs:list", &[]));
    assert!(runs_before.ok);
    assert_eq!(runs_before.data["runs"].as_array().unwrap().len(), 1);

    // 执行 logs:clear
    let env_clear = execute(&ctx, &req("logs:clear", &[]));
    assert!(env_clear.ok);
    assert_eq!(env_clear.data["deleted"], 3);

    // 验证日志文件已全被删除，且 logs:clear 本身不重新写日志文件
    let count = std::fs::read_dir(&logs_dir).unwrap().count();
    assert_eq!(count, 0, "全部日志文件应已被删除");

    // 验证 SQLite 任务记录不受影响
    let runs_after = execute(&ctx, &req("runs:list", &[]));
    assert!(runs_after.ok);
    assert_eq!(
        runs_after.data["runs"].as_array().unwrap().len(),
        1,
        "SQLite 任务记录必须保持"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn startup_maintenance_handles_missing_dir_and_prunes_safely() {
    let (base, ctx) = temp_ctx("startup-maintenance");
    let logs_dir = ctx.data_dir.join("logs");

    // 1. logs 目录不存在：不报错且不创建
    let pruned_missing = startup_maintenance(&ctx).unwrap();
    assert_eq!(pruned_missing, 0);
    assert!(!logs_dir.exists(), "目录不存在时不应创建目录");

    // 2. logs 目录存在且有过期文件：安全清理
    std::fs::create_dir_all(&logs_dir).unwrap();
    let today = Utc::now().date_naive();
    let old_date = (today - Duration::days(50)).format("%Y-%m-%d").to_string();
    let old_file = logs_dir.join(format!("agent-manager-{old_date}.jsonl"));
    std::fs::write(&old_file, "{}\n").unwrap();

    let pruned = startup_maintenance(&ctx).unwrap();
    assert_eq!(pruned, 1);
    assert!(!old_file.exists());

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn filename_helpers_test() {
    assert!(is_app_log_file("agent-manager-2026-09-09.jsonl"));
    assert!(!is_app_log_file("agent-manager-2026-9-9.jsonl"));
    assert!(!is_app_log_file("agent-manager-2026-02-31.jsonl")); // 非法日期
    assert!(!is_app_log_file("other-2026-09-09.jsonl"));
    assert!(!is_app_log_file("agent-manager-2026-09-09.txt"));

    let parsed = parse_log_file_date("agent-manager-2026-09-09.jsonl").unwrap();
    assert_eq!(parsed.to_string(), "2026-09-09");
}

#[test]
fn readonly_command_log_file_is_none_even_when_env_var_set() {
    let (base, ctx) = temp_ctx("readonly-env-var");
    std::env::set_var("AGENT_MANAGER_LOG_FILE", "/tmp/fake-log-path.jsonl");

    let env = execute(&ctx, &req("agents:status", &[]));
    assert!(env.ok);
    assert_eq!(env.log_file, None, "只读请求的 log_file 必须严格为 None");

    std::env::remove_var("AGENT_MANAGER_LOG_FILE");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
#[cfg(unix)]
fn logs_clear_propagates_filesystem_errors() {
    use std::os::unix::fs::PermissionsExt;

    let (base, ctx) = temp_ctx("clear-error");
    let logs_dir = ctx.data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir).unwrap();

    let target = logs_dir.join("agent-manager-2026-09-01.jsonl");
    std::fs::write(&target, "dummy").unwrap();

    // 将 logs 目录权限设为 0500（不可写），使 remove_file 失败
    std::fs::set_permissions(&logs_dir, std::fs::Permissions::from_mode(0o500)).unwrap();

    let env = execute(&ctx, &req("logs:clear", &[]));
    // 还原权限以便后续清理
    let _ = std::fs::set_permissions(&logs_dir, std::fs::Permissions::from_mode(0o700));

    assert!(!env.ok, "删除受保护文件应失败");
    let err = &env.errors[0];
    assert_eq!(err.code, "LOGS_CLEAR_ERROR");
    assert!(err.message.contains("删除日志文件失败"));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn diagnostic_log_entry_sanitizes_warnings() {
    let (base, ctx) = temp_ctx("warning-sanitize");
    std::fs::create_dir_all(&ctx.data_dir).unwrap();

    // 构造一个包含敏感信息的 Warning 并写入日志
    let entry = agent_manager_engine::native_logs::DiagnosticLogEntry {
        timestamp: Utc::now().to_rfc3339(),
        run_id: Some("run-warning-test".to_string()),
        command: "policy:set".to_string(),
        ok: true,
        duration_ms: 10,
        error_code: None,
        error_message: None,
        diagnostics: vec![agent_manager_engine::native_logs::SafeDiagnosticItem {
            code: "MIGRATION_WARNING".to_string(),
            message: agent_manager_engine::native_logs::sanitize_diagnostic_message(
                "Warning during token: secret_tok_12345 in Bearer abc-token-xyz",
            ),
        }],
    };

    let (write_res, _) =
        agent_manager_engine::native_logs::write_and_prune_diagnostic_log(&ctx, &entry);
    let log_path = write_res.unwrap();
    assert!(log_path.exists());

    let content = std::fs::read_to_string(&log_path).unwrap();
    assert!(!content.contains("secret_tok_12345"));
    assert!(!content.contains("abc-token-xyz"));
    assert!(content.contains("***"));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn mutating_preflight_failure_creates_logs_without_sqlite_store() {
    let (base, ctx) = temp_ctx("preflight-fail-log");
    // 尚未创建 data_dir，执行失败的写操作（例如参数非法的 agents:apply）
    let env = execute(&ctx, &req("agents:apply", &["--invalid-flag"]));
    assert!(!env.ok);
    assert!(env.log_file.is_some(), "失败的写操作必须记录诊断日志");

    let log_path = PathBuf::from(env.log_file.unwrap());
    assert!(log_path.exists());
    assert!(
        !store::db_path(&ctx).exists(),
        "失败写操作不得创建 SQLite 库"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn envelope_warnings_and_jsonl_sanitize_tokens_and_preserve_code_element() {
    let (base, ctx) = temp_ctx("warnings-sanitize");
    std::fs::create_dir_all(&ctx.data_dir).unwrap();

    let fake_token = "ghp_1234567890abcdef1234567890abcdef1234";
    let fake_bearer = "Bearer my_super_secret_token_val_999";
    let raw_msg = format!("Found token {fake_token} and header Authorization: {fake_bearer}");

    let mut diag = agent_manager_engine::diagnostic(
        "CUSTOM_WARNING",
        Some("policy:custom.test".to_string()),
        raw_msg,
    );

    // 验证集中脱敏逻辑：保留 code 和 element，message 彻底屏蔽 fake_token 和 fake_bearer
    diag.message = agent_manager_engine::native_logs::sanitize_diagnostic_message(&diag.message);
    assert_eq!(diag.code, "CUSTOM_WARNING");
    assert_eq!(diag.element, Some("policy:custom.test".to_string()));
    assert!(!diag.message.contains(fake_token));
    assert!(!diag.message.contains("my_super_secret_token_val_999"));
    assert!(diag.message.contains("gh***"));
    assert!(diag.message.contains("Bearer ***"));

    // 将包含脱敏 warning 的 entry 写入 JSONL 诊断日志
    let entry = agent_manager_engine::native_logs::DiagnosticLogEntry {
        timestamp: Utc::now().to_rfc3339(),
        run_id: Some("run-warning-check".to_string()),
        command: "policy:set".to_string(),
        ok: true,
        duration_ms: 5,
        error_code: None,
        error_message: None,
        diagnostics: vec![agent_manager_engine::native_logs::SafeDiagnosticItem {
            code: diag.code.clone(),
            message: diag.message.clone(),
        }],
    };
    let (write_res, _) =
        agent_manager_engine::native_logs::write_and_prune_diagnostic_log(&ctx, &entry);
    let log_path = write_res.unwrap();
    let content = std::fs::read_to_string(&log_path).unwrap();
    assert!(!content.contains(fake_token));
    assert!(!content.contains("my_super_secret_token_val_999"));
    assert!(content.contains("gh***"));
    assert!(content.contains("Bearer ***"));

    let _ = std::fs::remove_dir_all(&base);
}
