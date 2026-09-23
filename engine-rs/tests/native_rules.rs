//! rules 域：实际文件为准 + 模板 + 回填。
//!
//! 生效规则就是各助手的规则文件；status 输出实际全文与 base，apply 以 base 做 CAS
//! 写回实际文件；模板只写库；用模板/推荐/复制覆盖前先把对不上任何模板的现有内容
//! 另存为备份模板；旧 rule_sources 在写路径迁移成模板。

use agent_manager_engine::{execute, store, Context, Envelope, Request};
use fs2::FileExt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

const RECOMMENDED: &str = include_str!("../../agent-rule/rules/src/shared/core.md");

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

fn run(ctx: &Context, command: &str, args: &[&str]) -> Envelope {
    execute(ctx, &request(command, args))
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn status(ctx: &Context) -> Value {
    let env = run(ctx, "rules:status", &[]);
    assert!(env.ok, "{:?}", env.errors);
    env.data
}

fn element<'a>(status: &'a Value, id: &str) -> &'a Value {
    status["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .unwrap_or_else(|| panic!("缺少目标 {id}: {status}"))
}

fn base_of(ctx: &Context, id: &str) -> String {
    element(&status(ctx), id)["observed"]["base"]
        .as_str()
        .unwrap()
        .to_string()
}

fn templates(ctx: &Context) -> Vec<Value> {
    status(ctx)["templates"].as_array().unwrap().clone()
}

fn write_file(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn claude_md(ctx: &Context) -> PathBuf {
    ctx.home_dir.join(".claude/CLAUDE.md")
}

fn codex_md(ctx: &Context) -> PathBuf {
    ctx.home_dir.join(".codex/AGENTS.md")
}

fn db_rows(ctx: &Context, sql: &str) -> i64 {
    let conn = store::open_readonly(ctx).unwrap().expect("db must exist");
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn save_template(ctx: &Context, name: &str, body: &str) -> Value {
    let env = run(
        ctx,
        "rules:save-template",
        &["--name", name, "--value", body],
    );
    assert!(env.ok, "{:?}", env.errors);
    env.data["template"].clone()
}

#[test]
fn status_on_fresh_context_creates_nothing() {
    let (base, ctx) = temp_ctx("fresh");
    let env = run(&ctx, "rules:status", &["--offline"]);
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.command, "rules.status");
    assert!(!env.lock.held);
    let ids: Vec<&str> = env.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["rules:codex_global", "rules:claude_global"]);
    assert_eq!(env.data["recommended"]["name"], "内置推荐规则");
    assert_eq!(env.data["recommended"]["body"], RECOMMENDED);
    assert!(!ctx.data_dir.exists(), "status 不得创建 data_dir");
    assert!(!ctx.home_dir.exists(), "status 不得创建 home_dir");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn dot_command_form_is_normalized() {
    let (base, ctx) = temp_ctx("dotform");
    let env = run(&ctx, "rules.status", &[]);
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["domain"], "rules");
    let _ = std::fs::remove_dir_all(&base);
}

/// 外部修改后 status 展示新全文；旧 base 保存被拒且文件不变；新 base 保存成功。
#[test]
fn external_edit_is_shown_and_stale_base_is_rejected() {
    let (root, ctx) = temp_ctx("cas");
    write_file(&claude_md(&ctx), "第一版\n");
    let stale = base_of(&ctx, "rules:claude_global");

    write_file(&claude_md(&ctx), "外部改成第二版\n");
    let st = status(&ctx);
    let claude = element(&st, "rules:claude_global");
    assert_eq!(claude["state"], "managed-ok");
    assert_eq!(claude["observed"]["content"], "外部改成第二版\n");
    let fresh = claude["observed"]["base"].as_str().unwrap().to_string();
    assert_eq!(fresh, sha("外部改成第二版\n".as_bytes()));

    let env = run(
        &ctx,
        "rules:apply",
        &[
            "--only",
            "rules:claude_global",
            "--base",
            &stale,
            "--value",
            "我的新规则\n",
        ],
    );
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "RULE_ERROR");
    assert!(
        env.errors[0].message.contains("已在外部修改"),
        "{}",
        env.errors[0].message
    );
    assert!(!env.lock.held, "预检拒绝不取锁");
    assert_eq!(
        std::fs::read_to_string(claude_md(&ctx)).unwrap(),
        "外部改成第二版\n"
    );
    assert!(!store::db_path(&ctx).exists(), "拒绝时不建库");

    let env = run(
        &ctx,
        "rules:apply",
        &[
            "--only",
            "rules:claude_global",
            "--base",
            &fresh,
            "--value",
            "我的新规则\n",
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.lock.held, "写文件取 apply 锁");
    assert_eq!(
        env.data["steps"][0]["name"],
        "rules:write rules:claude_global"
    );
    assert_eq!(
        std::fs::read_to_string(claude_md(&ctx)).unwrap(),
        "我的新规则\n"
    );
    // 直接编辑不另存备份：用户看着原文改的
    assert!(templates(&ctx).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn missing_file_uses_missing_base_and_rejects_file_created_meanwhile() {
    let (root, ctx) = temp_ctx("missing-base");
    assert_eq!(base_of(&ctx, "rules:codex_global"), "missing");
    // 看的时候没有，写之前被别人建出来：同样判为外部修改
    write_file(&codex_md(&ctx), "别人刚写的\n");
    let env = run(
        &ctx,
        "rules:apply",
        &[
            "--only",
            "rules:codex_global",
            "--base",
            "missing",
            "--recommended",
        ],
    );
    assert!(!env.ok);
    assert!(env.errors[0].message.contains("已在外部修改"));
    assert_eq!(
        std::fs::read_to_string(codex_md(&ctx)).unwrap(),
        "别人刚写的\n"
    );

    std::fs::remove_file(codex_md(&ctx)).unwrap();
    let env = run(
        &ctx,
        "rules:apply",
        &[
            "--only",
            "rules:codex_global",
            "--base",
            "missing",
            "--value",
            "新建\n",
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(std::fs::read_to_string(codex_md(&ctx)).unwrap(), "新建\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(codex_md(&ctx))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 应用模板：对不上任何模板时先生成备份（正文与原文件逐字节一致）；对得上时不生成。
#[test]
fn applying_template_backs_up_unmatched_content_only() {
    let (root, ctx) = temp_ctx("template");
    let original = "# 手写规则\n\n  保留缩进与空行  \n";
    write_file(&claude_md(&ctx), original);
    let team = save_template(&ctx, "团队规则", "团队规则正文\n");
    let team_id = team["id"].as_str().unwrap();

    let base = base_of(&ctx, "rules:claude_global");
    let env = run(
        &ctx,
        "rules:apply",
        &[
            "--only",
            "rules:claude_global",
            "--base",
            &base,
            "--template",
            team_id,
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    let steps: Vec<&str> = env.data["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        steps,
        [
            "rules:backup rules:claude_global",
            "rules:write rules:claude_global"
        ]
    );
    assert_eq!(
        std::fs::read_to_string(claude_md(&ctx)).unwrap(),
        "团队规则正文\n"
    );

    let list = templates(&ctx);
    let backup = list
        .iter()
        .find(|t| t["origin"] == "backup")
        .expect("应生成备份模板");
    assert_eq!(
        backup["body"].as_str().unwrap().as_bytes(),
        original.as_bytes()
    );
    let name = backup["name"].as_str().unwrap();
    assert!(name.contains("Claude Code 全局规则"), "名称含目标: {name}");
    assert!(name.starts_with("备份 · "), "{name}");
    // status 回读：当前内容与团队模板一致
    let st = status(&ctx);
    assert_eq!(
        element(&st, "rules:claude_global")["observed"]["matched_template"]["id"],
        team_id
    );

    // 再切回备份模板：当前内容等于「团队规则」模板，不再生成新备份
    let backup_id = backup["id"].as_str().unwrap().to_string();
    let base = base_of(&ctx, "rules:claude_global");
    let env = run(
        &ctx,
        "rules:apply",
        &[
            "--only",
            "rules:claude_global",
            "--base",
            &base,
            "--template",
            &backup_id,
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["steps"].as_array().unwrap().len(), 1);
    assert_eq!(templates(&ctx).len(), 2, "匹配模板时不生成备份");
    assert_eq!(
        std::fs::read(claude_md(&ctx)).unwrap(),
        original.as_bytes(),
        "切回备份后逐字节还原"
    );

    // 同内容再应用一次：什么都不写
    let base = base_of(&ctx, "rules:claude_global");
    let env = run(
        &ctx,
        "rules:apply",
        &[
            "--only",
            "rules:claude_global",
            "--base",
            &base,
            "--template",
            &backup_id,
        ],
    );
    assert!(env.ok);
    assert_eq!(env.data["steps"], serde_json::json!([]));
    assert_eq!(
        env.data["skipped"],
        serde_json::json!(["rules:claude_global"])
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 内置推荐：应用前后回读；推荐正文从不入库。
#[test]
fn recommended_is_applied_but_never_stored() {
    let (root, ctx) = temp_ctx("recommended");
    write_file(&codex_md(&ctx), "旧的 Codex 规则\n");
    let before = status(&ctx);
    let codex = element(&before, "rules:codex_global");
    assert_eq!(codex["observed"]["matches_recommended"], false);
    let base = codex["observed"]["base"].as_str().unwrap().to_string();

    let env = run(
        &ctx,
        "rules:apply",
        &[
            "--only",
            "rules:codex_global",
            "--base",
            &base,
            "--recommended",
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        std::fs::read_to_string(codex_md(&ctx)).unwrap(),
        RECOMMENDED
    );
    let after = status(&ctx);
    let codex = element(&after, "rules:codex_global");
    assert_eq!(codex["observed"]["content"], RECOMMENDED);
    assert_eq!(codex["observed"]["matches_recommended"], true);

    // 库里只有旧内容的备份，没有任何一行是推荐正文
    let list = templates(&ctx);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["body"], "旧的 Codex 规则\n");
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let stored_recommended: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM rule_templates WHERE body = ?1",
            [RECOMMENDED],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored_recommended, 0, "内置推荐不得入库");
    drop(conn);

    // 目标已是推荐内容时再用模板覆盖：推荐内容不当备份存进库
    let tpl = save_template(&ctx, "另一个", "另一个\n");
    let base = base_of(&ctx, "rules:codex_global");
    let env = run(
        &ctx,
        "rules:apply",
        &[
            "--only",
            "rules:codex_global",
            "--base",
            &base,
            "--template",
            tpl["id"].as_str().unwrap(),
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let stored_recommended: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM rule_templates WHERE body = ?1",
            [RECOMMENDED],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored_recommended, 0, "推荐内容被覆盖时也不入库");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn recommended_not_applied_leaves_no_db_at_all() {
    let (root, ctx) = temp_ctx("recommended-untouched");
    write_file(&claude_md(&ctx), RECOMMENDED);
    let st = status(&ctx);
    assert_eq!(
        element(&st, "rules:claude_global")["observed"]["matches_recommended"],
        true,
        "旧版本按默认写入的文件被识别为内置推荐"
    );
    assert!(
        !store::db_path(&ctx).exists(),
        "未应用时库都不存在，更没有推荐行"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 复制 Claude → Codex：Codex 原内容先备份，写入后逐字节等于 Claude 实际内容。
#[test]
fn copy_between_targets_backs_up_destination() {
    let (root, ctx) = temp_ctx("copy");
    let claude_text = "Claude 的规则\n- 一条\n";
    let codex_text = "Codex 原来的规则\n";
    write_file(&claude_md(&ctx), claude_text);
    write_file(&codex_md(&ctx), codex_text);
    let base = base_of(&ctx, "rules:codex_global");
    let env = run(
        &ctx,
        "rules:apply",
        &[
            "--only",
            "rules:codex_global",
            "--base",
            &base,
            "--from",
            "rules:claude_global",
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        std::fs::read(codex_md(&ctx)).unwrap(),
        std::fs::read(claude_md(&ctx)).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(claude_md(&ctx)).unwrap(),
        claude_text,
        "来源不动"
    );
    let list = templates(&ctx);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["origin"], "backup");
    assert_eq!(list[0]["body"], codex_text);
    assert!(list[0]["name"].as_str().unwrap().contains("Codex 全局规则"));

    // 审计只有元数据，不含任何正文
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let detail: String = conn
        .query_row(
            "SELECT detail_json FROM operation_logs WHERE operation_type = 'rules:apply'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(detail.contains("rules:claude_global"));
    assert!(!detail.contains("Claude 的规则") && !detail.contains("Codex 原来的规则"));

    // 来源没有文件：拒绝，不写
    std::fs::remove_file(claude_md(&ctx)).unwrap();
    let base = base_of(&ctx, "rules:codex_global");
    let env = run(
        &ctx,
        "rules:apply",
        &[
            "--only",
            "rules:codex_global",
            "--base",
            &base,
            "--from",
            "rules:claude_global",
        ],
    );
    assert!(!env.ok);
    assert!(env.errors[0].message.contains("还没有规则文件"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn backup_refuses_content_that_looks_like_a_secret() {
    let (root, ctx) = temp_ctx("secret-backup");
    let secret_line = format!("token sk-{}\n", "a".repeat(24));
    write_file(&claude_md(&ctx), &secret_line);
    let base = base_of(&ctx, "rules:claude_global");
    let env = run(
        &ctx,
        "rules:apply",
        &[
            "--only",
            "rules:claude_global",
            "--base",
            &base,
            "--recommended",
        ],
    );
    assert!(!env.ok, "疑似凭据不能进库，也就不能不备份就覆盖");
    assert!(!serde_json::to_string(&env).unwrap().contains("sk-aaaa"));
    assert_eq!(
        std::fs::read_to_string(claude_md(&ctx)).unwrap(),
        secret_line
    );
    assert!(!store::db_path(&ctx).exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn templates_save_edit_delete_only_touch_the_db() {
    let (root, ctx) = temp_ctx("templates");
    write_file(&claude_md(&ctx), "哨兵\n");
    let created = save_template(&ctx, "  草稿 ", "v1\n");
    assert_eq!(created["name"], "草稿");
    assert_eq!(created["origin"], "user");
    let id = created["id"].as_str().unwrap().to_string();
    let rev1 = created["revision"].as_str().unwrap().to_string();

    // 同名拒绝
    let dup = run(
        &ctx,
        "rules:save-template",
        &["--name", "草稿", "--value", "x"],
    );
    assert!(!dup.ok);
    assert!(dup.errors[0].message.contains("同名"));

    let env = run(
        &ctx,
        "rules:save-template",
        &[
            "--template",
            &id,
            "--revision",
            &rev1,
            "--name",
            "定稿",
            "--value",
            "v2\n",
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["template"]["body"], "v2\n");
    // 过期 revision 拒绝，内容不变
    let stale = run(
        &ctx,
        "rules:save-template",
        &[
            "--template",
            &id,
            "--revision",
            &rev1,
            "--name",
            "定稿",
            "--value",
            "v3\n",
        ],
    );
    assert!(!stale.ok);
    assert!(stale.errors[0].message.contains("已被修改"));
    assert_eq!(templates(&ctx)[0]["body"], "v2\n");

    // 疑似凭据拒绝且不回显
    let secret = format!("sk-{}", "b".repeat(24));
    let env = run(
        &ctx,
        "rules:save-template",
        &["--name", "坏", "--value", &secret],
    );
    assert!(!env.ok);
    assert!(!serde_json::to_string(&env).unwrap().contains(&secret));

    let env = run(&ctx, "rules:delete-template", &["--template", &id]);
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["deleted"], id.as_str());
    assert!(templates(&ctx).is_empty());
    let env = run(&ctx, "rules:delete-template", &["--template", &id]);
    assert!(!env.ok, "删除不存在的模板报错");

    // 只写库：规则文件不动，也不取 apply 锁
    assert_eq!(std::fs::read_to_string(claude_md(&ctx)).unwrap(), "哨兵\n");
    assert!(!ctx.data_dir.join("agent-manager.apply.lock").exists());
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let details: Vec<String> = conn
        .prepare("SELECT detail_json FROM operation_logs WHERE operation_type LIKE 'rules:%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(details.len(), 3);
    for detail in details {
        assert!(
            !detail.contains("v2") && !detail.contains("草稿"),
            "{detail}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// status 零写：已有库与文件时，status 前后库字节与表集合不变。
#[test]
fn status_is_zero_write_on_existing_db() {
    let (root, ctx) = temp_ctx("zero-write");
    write_file(&claude_md(&ctx), "内容\n");
    save_template(&ctx, "一个", "正文\n");
    let db = store::db_path(&ctx);
    let before = std::fs::read(&db).unwrap();
    let file_before = std::fs::metadata(claude_md(&ctx))
        .unwrap()
        .modified()
        .unwrap();
    for _ in 0..2 {
        status(&ctx);
    }
    assert_eq!(std::fs::read(&db).unwrap(), before, "status 不得改库");
    assert_eq!(
        std::fs::metadata(claude_md(&ctx))
            .unwrap()
            .modified()
            .unwrap(),
        file_before
    );
    let _ = std::fs::remove_dir_all(&root);
}

fn save_profile(ctx: &Context, id: &str, value: &str) {
    let env = run(
        ctx,
        "policy:set",
        &[
            "--layer",
            "local",
            "--key",
            &format!("agents.definitions.{id}"),
            "--value",
            value,
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
}

/// 实例规则文件是规则目标；profiles:apply 不再写它。
#[test]
fn profile_rules_file_is_a_target_and_profiles_apply_leaves_it_alone() {
    let (root, ctx) = temp_ctx("profile");
    save_profile(
        &ctx,
        "cx-work",
        r#"{"display_name":"Codex 工作","bin":"codex","profile":{"base_agent":"codex","config_dir":"~/.codex-work"},"_delete":false,"_replace":true}"#,
    );
    // 目录还不存在：可见但不能写
    let st = status(&ctx);
    let target = element(&st, "rules:profile:cx-work");
    assert_eq!(target["spec"]["target"], "~/.codex-work/AGENTS.md");
    assert_eq!(target["state"], "missing");
    assert!(target["observed"]["blocked_reason"].as_str().is_some());
    assert_eq!(target["actions"], serde_json::json!([]));

    // profiles:apply 建目录，但不生成规则文件
    let env = run(&ctx, "profiles:apply", &["--only", "profiles:cx-work"]);
    assert!(env.ok, "{:?}", env.errors);
    let rules = ctx.home_dir.join(".codex-work/AGENTS.md");
    assert!(!rules.exists(), "新建实例不自动写规则文件");

    // 从主 Codex 复制过来
    write_file(&codex_md(&ctx), "主 Codex 规则\n");
    let base = base_of(&ctx, "rules:profile:cx-work");
    assert_eq!(base, "missing");
    let env = run(
        &ctx,
        "rules:apply",
        &[
            "--only",
            "rules:profile:cx-work",
            "--base",
            &base,
            "--from",
            "rules:codex_global",
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(std::fs::read_to_string(&rules).unwrap(), "主 Codex 规则\n");

    // 用户改了实例规则后 profiles:apply：规则文件字节不变，也不因它拒绝
    write_file(&rules, "实例自己的规则\n");
    let before = std::fs::read(&rules).unwrap();
    let env = run(&ctx, "profiles:apply", &["--only", "profiles:cx-work"]);
    assert!(env.ok, "规则文件不再是实例域的写入面: {:?}", env.errors);
    assert_eq!(std::fs::read(&rules).unwrap(), before);
    let st = status(&ctx);
    assert_eq!(
        element(&st, "rules:profile:cx-work")["observed"]["content"],
        "实例自己的规则\n"
    );
    let _ = std::fs::remove_dir_all(&root);
}

fn legacy_db(ctx: &Context, rows: &[(&str, &str)]) {
    let conn = store::open_write(ctx).unwrap();
    conn.execute_batch(
        "CREATE TABLE rule_sources (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            revision TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .unwrap();
    for (key, value) in rows {
        conn.execute(
            "INSERT INTO rule_sources (key, value, revision, updated_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![key, value, sha(value.as_bytes()), "2026-09-12T00:00:00Z"],
        )
        .unwrap();
    }
}

fn template_summary(list: &[Value]) -> Vec<(String, String, String)> {
    list.iter()
        .map(|t| {
            (
                t["id"].as_str().unwrap().to_string(),
                t["name"].as_str().unwrap().to_string(),
                t["body"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// 旧库迁移①：来源从未改过（存过但等于内置）→ 不生成模板。
#[test]
fn legacy_sources_never_changed_yield_no_templates() {
    let (root, ctx) = temp_ctx("legacy-none");
    legacy_db(&ctx, &[("rules.shared", RECOMMENDED), ("rules.codex", "")]);
    assert!(templates(&ctx).is_empty(), "读路径不推导");
    save_template(&ctx, "触发写路径", "x\n");
    let list = templates(&ctx);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["name"], "触发写路径");
    let _ = std::fs::remove_dir_all(&root);
}

/// 旧库迁移②：改过 shared → 两个旧来源模板；读路径即时推导与写路径迁移结果一致；
/// 迁移不改实际文件；迁移后删掉的模板不会回来。
#[test]
fn legacy_changed_shared_becomes_two_templates() {
    let (root, ctx) = temp_ctx("legacy-shared");
    legacy_db(&ctx, &[("rules.shared", "团队共享\n\n第二行  \n")]);
    write_file(&claude_md(&ctx), "旧版本写出的文件\n");
    let db_before = std::fs::read(store::db_path(&ctx)).unwrap();

    let derived = templates(&ctx);
    assert_eq!(
        std::fs::read(store::db_path(&ctx)).unwrap(),
        db_before,
        "读路径推导零写"
    );
    assert_eq!(
        template_summary(&derived),
        [
            (
                "legacy-claude".into(),
                "旧规则来源（Claude）".into(),
                "团队共享\n第二行\n".into()
            ),
            (
                "legacy-codex".into(),
                "旧规则来源（Codex）".into(),
                "团队共享\n第二行\n".into()
            ),
        ]
    );
    for t in &derived {
        assert_eq!(t["origin"], "legacy");
    }

    // 触发写路径：迁移落库，结果与推导一致
    let env = run(
        &ctx,
        "rules:delete-template",
        &["--template", "legacy-codex"],
    );
    assert!(env.ok, "{:?}", env.errors);
    let after = templates(&ctx);
    assert_eq!(template_summary(&after), template_summary(&derived[..1]));
    assert_eq!(
        db_rows(&ctx, "SELECT COUNT(*) FROM rule_templates"),
        1,
        "旧来源模板已真实入库"
    );
    assert_eq!(
        db_rows(&ctx, "SELECT COUNT(*) FROM rule_sources"),
        1,
        "旧表原样保留"
    );
    assert_eq!(
        std::fs::read_to_string(claude_md(&ctx)).unwrap(),
        "旧版本写出的文件\n",
        "迁移不改实际文件"
    );
    // 再走一次写路径：删掉的不会回来
    save_template(&ctx, "新的", "y\n");
    let ids: Vec<String> = templates(&ctx)
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&"legacy-claude".to_string()));
    assert!(!ids.contains(&"legacy-codex".to_string()));
    let _ = std::fs::remove_dir_all(&root);
}

/// 旧库迁移③：只改过 Claude adapter → 只生成 Claude 旧来源模板（共享取内置）。
#[test]
fn legacy_changed_adapter_becomes_one_template() {
    let (root, ctx) = temp_ctx("legacy-adapter");
    legacy_db(&ctx, &[("rules.claude", "只给 Claude\n")]);
    let list = templates(&ctx);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], "legacy-claude");
    assert_eq!(list[0]["body"], format!("{RECOMMENDED}只给 Claude\n"));

    // 应用它：目标原内容若对不上模板会先备份
    write_file(&claude_md(&ctx), RECOMMENDED);
    let base = base_of(&ctx, "rules:claude_global");
    let env = run(
        &ctx,
        "rules:apply",
        &[
            "--only",
            "rules:claude_global",
            "--base",
            &base,
            "--template",
            "legacy-claude",
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        std::fs::read_to_string(claude_md(&ctx)).unwrap(),
        format!("{RECOMMENDED}只给 Claude\n")
    );
    assert_eq!(templates(&ctx).len(), 1, "原内容是内置推荐，不另存备份");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn old_bindings_table_is_no_longer_a_write_gate() {
    let (root, ctx) = temp_ctx("old-bindings");
    {
        let conn = store::open_write(&ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE rule_target_bindings (
                target_id TEXT PRIMARY KEY NOT NULL,
                applied_digest TEXT NOT NULL,
                shared_revision TEXT NOT NULL,
                adapter_revision TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            INSERT INTO rule_target_bindings VALUES ('rules:claude_global', 'x', 'y', 'z', 't');",
        )
        .unwrap();
    }
    write_file(&claude_md(&ctx), "外部写的\n");
    let st = status(&ctx);
    assert_eq!(element(&st, "rules:claude_global")["state"], "managed-ok");
    let base = base_of(&ctx, "rules:claude_global");
    let env = run(
        &ctx,
        "rules:apply",
        &[
            "--only",
            "rules:claude_global",
            "--base",
            &base,
            "--value",
            "改好\n",
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn retired_and_invalid_args_are_rejected_without_writes() {
    let (root, ctx) = temp_ctx("invalid");
    let marker = "RULES-ARG-MARKER-e81c";
    for args in [
        vec![],
        vec!["--dry-run"],
        vec!["--only", "rules:claude_global", "--allow-takeover"],
        vec!["--only", "rules:claude_global", "--base", "missing"],
        vec!["--only", marker, "--base", "missing", "--recommended"],
        vec![
            "--only",
            "rules:claude_global",
            "--base",
            "missing",
            "--recommended",
            marker,
        ],
        vec![
            "--only",
            "rules:profile:nobody",
            "--base",
            "missing",
            "--recommended",
        ],
        vec![
            "--only",
            "rules:claude_global",
            "--base",
            "missing",
            "--template",
            "tpl-nope",
        ],
    ] {
        let env = run(&ctx, "rules:apply", &args);
        assert!(!env.ok, "{args:?} 必须失败");
        assert_eq!(env.errors[0].code, "RULE_ERROR", "{args:?}");
        assert!(!env.lock.held, "{args:?}");
        assert!(
            !serde_json::to_string(&env).unwrap().contains(marker),
            "错误 envelope 不得回显传入 token: {args:?}"
        );
    }
    assert!(!ctx.home_dir.exists());
    assert!(!store::db_path(&ctx).exists());
    assert!(!ctx.data_dir.join("agent-manager.apply.lock").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn apply_rejects_contended_lock_without_touching_target() {
    let (root, ctx) = temp_ctx("lock");
    std::fs::create_dir_all(&ctx.data_dir).unwrap();
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(ctx.data_dir.join("agent-manager.apply.lock"))
        .unwrap();
    lock_file.lock_exclusive().unwrap();
    let worker_ctx = Context {
        data_dir: ctx.data_dir.clone(),
        home_dir: ctx.home_dir.clone(),
    };
    let env = std::thread::spawn(move || {
        execute(
            &worker_ctx,
            &request(
                "rules:apply",
                &[
                    "--only",
                    "rules:codex_global",
                    "--base",
                    "missing",
                    "--recommended",
                ],
            ),
        )
    })
    .join()
    .unwrap();
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "RULE_ERROR");
    assert!(!ctx.home_dir.exists(), "锁竞争失败不得写目标");
    let _ = std::fs::remove_dir_all(&root);
}

/// status.schema.json 的 status 数据与规则两个字段都是 additionalProperties=false：
/// 输出的键必须与契约声明一致。
#[test]
fn status_matches_contract_schema() {
    let (root, ctx) = temp_ctx("schema");
    save_template(&ctx, "契约", "x\n");
    let data = status(&ctx);
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../contract/engine-v1/status.schema.json");
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let keys = |value: &Value| -> std::collections::BTreeSet<String> {
        value.as_object().unwrap().keys().cloned().collect()
    };
    let strings = |value: &Value| -> std::collections::BTreeSet<String> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    };
    assert!(keys(&data).is_subset(&keys(&schema["properties"])));
    let template_def = &schema["$defs"]["rule_template"];
    for item in data["templates"].as_array().unwrap() {
        assert_eq!(keys(item), keys(&template_def["properties"]));
        assert_eq!(keys(item), strings(&template_def["required"]));
    }
    let recommended_def = &schema["properties"]["recommended"];
    assert_eq!(
        keys(&data["recommended"]),
        keys(&recommended_def["properties"])
    );
    let states = strings(&schema["$defs"]["element"]["properties"]["state"]["enum"]);
    for element in data["elements"].as_array().unwrap() {
        assert!(states.contains(element["state"].as_str().unwrap()));
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 模板表被外部弄坏：规则读写 fail-closed，但旧来源迁移不挡住其他域的写入。
#[test]
fn corrupt_template_table_fails_closed_without_blocking_other_domains() {
    let (root, ctx) = temp_ctx("corrupt-templates");
    legacy_db(&ctx, &[("rules.shared", "改过的共享\n")]);
    {
        let conn = rusqlite::Connection::open(store::db_path(&ctx)).unwrap();
        conn.execute_batch("CREATE TABLE rule_templates (id TEXT PRIMARY KEY, note TEXT)")
            .unwrap();
    }
    let env = run(&ctx, "rules:status", &[]);
    assert!(!env.ok, "模板表损坏时 status 报错，不猜");
    let env = run(
        &ctx,
        "policy:set",
        &[
            "--layer",
            "local",
            "--key",
            "mcp.enabled",
            "--value",
            "true",
        ],
    );
    assert!(env.ok, "其他域写入不受影响: {:?}", env.errors);
    let _ = std::fs::remove_dir_all(&root);
}
