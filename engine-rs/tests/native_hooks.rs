//! hooks 域：实际配置为准 + 清理建议 + 旧版本保存的 hooks（未生效）。
//!
//! 生效 hooks 就是各助手的实际配置：status 输出实际 hooks 与 base，apply 以 base 做 CAS
//! 写回实际文件（Claude 只替换 settings.json 的 hooks 键）；内置 removed_signatures 只作
//! 清理建议；旧 hook_sources 里没生效的 hooks 作为建议出现，可加入或丢弃。

use agent_manager_engine::{execute, store, Context, Envelope, Request};
use fs2::FileExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

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

fn run(ctx: &Context, command: &str, args: &[&str]) -> Envelope {
    execute(ctx, &request(command, args))
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn status(ctx: &Context) -> Value {
    let env = run(ctx, "hooks:status", &[]);
    assert!(env.ok, "{:?}", env.errors);
    env.data
}

fn element(status: &Value, id: &str) -> Value {
    status["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("缺少目标 {id}: {status}"))
}

fn observed(ctx: &Context, id: &str) -> Value {
    element(&status(ctx), id)["observed"].clone()
}

fn base_of(ctx: &Context, id: &str) -> String {
    observed(ctx, id)["base"].as_str().unwrap().to_string()
}

fn write_json(path: &Path, value: &Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

/// 实例写入基准（profiles:status 给出的实际内容摘要）。
fn profile_base(ctx: &Context, id: &str) -> String {
    let st = run(ctx, "profiles:status", &[]);
    st.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .unwrap_or_else(|| panic!("缺实例 {id}"))["observed"]["base"]
        .as_str()
        .unwrap_or_else(|| panic!("{id} 没有 base"))
        .to_string()
}

/// 按当前实际内容写入实例。
fn apply_profile(ctx: &Context, id: &str) -> Envelope {
    let base = profile_base(ctx, id);
    run(ctx, "profiles:apply", &["--only", id, "--base", &base])
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn settings(ctx: &Context) -> PathBuf {
    ctx.home_dir.join(".claude/settings.json")
}

fn codex_hooks(ctx: &Context) -> PathBuf {
    ctx.home_dir.join(".codex/hooks.json")
}

fn hook(command: &str) -> Value {
    json!({"type": "command", "command": command})
}

/// 去掉 hooks 键后的其余内容：用来断言「其他键语义不变」。
fn without_hooks(value: &Value) -> Value {
    let mut obj = value.as_object().unwrap().clone();
    obj.remove("hooks");
    Value::Object(obj)
}

fn apply(ctx: &Context, id: &str, base: &str, source: &[&str]) -> Envelope {
    let mut args = vec!["--only", id, "--base", base];
    args.extend_from_slice(source);
    run(ctx, "hooks:apply", &args)
}

fn apply_value(ctx: &Context, id: &str, base: &str, hooks: &Value) -> Envelope {
    apply(ctx, id, base, &["--value", &hooks.to_string()])
}

fn write_legacy_row(ctx: &Context, key: &str, value: &Value) {
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
    let text = value.to_string();
    conn.execute(
        "INSERT INTO hook_sources (key, value_json, revision, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, revision = excluded.revision",
        rusqlite::params![key, text, sha(text.as_bytes()), "2026-09-12T00:00:00Z"],
    )
    .unwrap();
}

fn legacy_rows(ctx: &Context) -> Vec<String> {
    let conn = store::open_readonly(ctx).unwrap().unwrap();
    let mut stmt = conn
        .prepare("SELECT key FROM hook_sources ORDER BY key")
        .unwrap();
    stmt.query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn commands(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|item| item["command"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn status_on_fresh_context_is_zero_write_with_two_targets() {
    let (root, ctx) = temp_ctx("fresh");
    let env = run(&ctx, "hooks:status", &["--offline"]);
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.mode.offline);
    assert!(!env.lock.held);
    let elements = env.data["elements"].as_array().unwrap();
    let ids: Vec<&str> = elements.iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["hooks:claude", "hooks:codex"]);
    assert_eq!(elements[0]["spec"]["target"], "~/.claude/settings.json");
    assert_eq!(elements[0]["spec"]["scope"], "hooks-key");
    assert_eq!(elements[1]["spec"]["target"], "~/.codex/hooks.json");
    assert_eq!(elements[1]["spec"]["scope"], "whole-file");
    for el in elements {
        assert_eq!(el["state"], "missing");
        let observed = &el["observed"];
        assert_eq!(observed["base"], "missing");
        assert_eq!(observed["hooks"], json!({}));
        assert_eq!(observed["hook_count"], 0);
        assert_eq!(observed["cleanup"], json!([]));
        assert_eq!(observed["legacy"], json!([]));
        assert_eq!(el["actions"][0]["id"], "write");
    }
    // 契约：status 顶层只用已声明的键
    let schema: Value = serde_json::from_str(
        &std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../contract/engine-v1/status.schema.json"),
        )
        .unwrap(),
    )
    .unwrap();
    for key in env.data.as_object().unwrap().keys() {
        assert!(schema["properties"].get(key).is_some(), "{key}");
    }
    assert!(!ctx.data_dir.exists(), "status 不得创建 data_dir");
    assert!(!ctx.home_dir.exists(), "status 不得创建 home_dir");
    let _ = std::fs::remove_dir_all(&root);
}

/// 外部在 settings.json 加了一个 hook：status 如实展示；旧 base 被拒且文件不变；
/// 带新 base 编辑另一个 hook 后，外部加的那个仍在，settings.json 其他键原样保留。
#[test]
fn external_hook_is_shown_stale_base_rejected_and_edit_keeps_everything_else() {
    let (root, ctx) = temp_ctx("external");
    let initial = json!({
        "env": {"ANTHROPIC_MODEL": "claude-opus-5", "MY_VAR": "1"},
        "permissions": {"allow": ["Bash"], "defaultMode": "manual"},
        "hooks": {"Stop": [{"hooks": [hook("echo ours")]}]},
        "statusLine": {"type": "command", "command": "status.sh"}
    });
    write_json(&settings(&ctx), &initial);
    let stale = base_of(&ctx, "hooks:claude");

    // 外部加了一个 hook
    let mut external = initial.clone();
    external["hooks"]["PreToolUse"] = json!([{"matcher": "Bash", "hooks": [{"type": "command", "command": "guard.sh", "timeout": 30}]}]);
    write_json(&settings(&ctx), &external);
    let shown = observed(&ctx, "hooks:claude");
    assert_eq!(shown["hooks"], external["hooks"], "status 展示实际 hooks");
    assert_eq!(shown["hook_count"], 2);
    assert_eq!(
        element(&status(&ctx), "hooks:claude")["state"],
        "managed-ok"
    );

    // 旧 base：拒绝，文件字节不变
    let before = std::fs::read(settings(&ctx)).unwrap();
    let edited = json!({
        "Stop": [{"hooks": [hook("echo ours v2")]}],
        "PreToolUse": external["hooks"]["PreToolUse"].clone()
    });
    let refused = apply_value(&ctx, "hooks:claude", &stale, &edited);
    assert!(!refused.ok);
    assert_eq!(refused.errors[0].code, "HOOK_ERROR");
    assert!(
        refused.errors[0].message.contains("已在外部修改"),
        "{:?}",
        refused.errors
    );
    assert_eq!(std::fs::read(settings(&ctx)).unwrap(), before);

    // 用户改 settings.json 的其他键不影响 hooks 的 base
    let fresh = base_of(&ctx, "hooks:claude");
    let mut other_edit = external.clone();
    other_edit["model"] = json!("claude-sonnet-5");
    write_json(&settings(&ctx), &other_edit);
    assert_eq!(base_of(&ctx, "hooks:claude"), fresh);

    let env = apply_value(&ctx, "hooks:claude", &fresh, &edited);
    assert!(env.ok, "{:?}", env.errors);
    let after = read_json(&settings(&ctx));
    assert_eq!(
        after["hooks"], edited,
        "编辑的那个改了，外部加的仍在（含 timeout）"
    );
    assert_eq!(
        without_hooks(&after),
        without_hooks(&other_edit),
        "其他键语义不变"
    );
    let keys: Vec<&String> = after.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        ["env", "permissions", "hooks", "statusLine", "model"],
        "hooks 键原位"
    );

    // 审计只记 id 与摘要，不含命令
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let detail: String = conn
        .query_row(
            "SELECT detail_json FROM operation_logs WHERE operation_type = 'hooks:apply'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!detail.contains("echo ours"), "{detail}");
    assert!(detail.contains(&fresh));
    let _ = std::fs::remove_dir_all(&root);
}

/// Codex hooks.json 外部改过：直接展示实际内容，不再判「非受管」；编辑保留其他根键。
#[test]
fn codex_external_content_is_shown_and_editable() {
    let (root, ctx) = temp_ctx("codex");
    let foreign = json!({
        "hooks": {"SessionStart": [{"hooks": [hook("sh user-own.sh")]}]},
        "foreign": true
    });
    write_json(&codex_hooks(&ctx), &foreign);
    let st = status(&ctx);
    let codex = element(&st, "hooks:codex");
    assert_eq!(codex["state"], "managed-ok");
    assert_eq!(codex["observed"]["blocked_reason"], Value::Null);
    assert_eq!(codex["observed"]["hooks"], foreign["hooks"]);
    assert_eq!(
        codex["observed"]["base"],
        sha(&std::fs::read(codex_hooks(&ctx)).unwrap())
    );
    let base = codex["observed"]["base"].as_str().unwrap().to_string();
    let next = json!({"SessionStart": [{"hooks": [hook("sh user-own.sh"), hook("echo added")]}]});
    let env = apply_value(&ctx, "hooks:codex", &base, &next);
    assert!(env.ok, "{:?}", env.errors);
    let after = read_json(&codex_hooks(&ctx));
    assert_eq!(after["hooks"], next);
    assert_eq!(after["foreign"], true, "hooks.json 的其他根键原样保留");

    // 整份字节变了就是外部修改（Codex 的 base 覆盖整个文件）
    let base = base_of(&ctx, "hooks:codex");
    let mut touched = after.clone();
    touched["foreign"] = json!(false);
    write_json(&codex_hooks(&ctx), &touched);
    let refused = apply_value(&ctx, "hooks:codex", &base, &json!({}));
    assert!(!refused.ok);
    assert!(refused.errors[0].message.contains("已在外部修改"));
    let _ = std::fs::remove_dir_all(&root);
}

/// 清理建议：实际 hooks 命中内置旧签名才出现；确认清理一条只移除那一条；不算待同步。
#[test]
fn cleanup_suggestion_removes_only_that_hook() {
    let (root, ctx) = temp_ctx("cleanup");
    let st = status(&ctx);
    assert_eq!(
        element(&st, "hooks:claude")["observed"]["cleanup"],
        json!([]),
        "无匹配不显示"
    );

    let old_check = "bash /repo/agent-rule/hooks/check-rules-build.sh";
    let old_block = "sh /repo/agent-rule/hooks/block-dist-edits.sh";
    write_json(
        &settings(&ctx),
        &json!({
            "model": "m",
            "hooks": {
                "SubagentStop": [{"hooks": [hook(old_check)]}],
                "PreToolUse": [{"matcher": "Edit|MultiEdit|Write", "hooks": [hook(old_block), hook("keep.sh")]}]
            }
        }),
    );
    let claude = element(&status(&ctx), "hooks:claude");
    assert_eq!(claude["state"], "managed-ok", "清理建议不改变状态");
    let cleanup = claude["observed"]["cleanup"].clone();
    assert_eq!(commands(&cleanup), [old_check, old_block]);
    let block_id = cleanup[1]["id"].as_str().unwrap().to_string();
    let base = claude["observed"]["base"].as_str().unwrap().to_string();

    let env = apply(&ctx, "hooks:claude", &base, &["--cleanup", &block_id]);
    assert!(env.ok, "{:?}", env.errors);
    let after = read_json(&settings(&ctx));
    assert_eq!(
        after["hooks"],
        json!({
            "SubagentStop": [{"hooks": [hook(old_check)]}],
            "PreToolUse": [{"matcher": "Edit|MultiEdit|Write", "hooks": [hook("keep.sh")]}]
        }),
        "只移除确认清理的那一条"
    );
    assert_eq!(after["model"], "m");
    let left = observed(&ctx, "hooks:claude")["cleanup"].clone();
    assert_eq!(commands(&left), [old_check]);

    // 已清理的建议再点一次：建议已不存在
    let base = base_of(&ctx, "hooks:claude");
    let again = apply(&ctx, "hooks:claude", &base, &["--cleanup", &block_id]);
    assert!(!again.ok);
    assert!(again.errors[0].message.contains("已不存在"));
    let _ = std::fs::remove_dir_all(&root);
}

/// 旧库迁移：库中存过、实际没有的 hooks 显示为未生效建议；读路径零写；加入后出现在实际
/// 配置里并从库里清掉；丢弃后库中无该来源；已在实际里的不重复提示，之后删掉也不再冒出来。
#[test]
fn legacy_saved_hooks_can_be_added_or_discarded() {
    let (root, ctx) = temp_ctx("legacy");
    write_legacy_row(
        &ctx,
        "hooks.claude",
        &json!({
            "hooks": {
                "Stop": [{"matcher": "", "hooks": [{"type": "command", "command": "echo x", "timeout": 5}]}],
                "SessionStart": [{"hooks": [hook("echo y"), hook("echo z")]}]
            },
            "removed_signatures": []
        }),
    );
    write_legacy_row(
        &ctx,
        "hooks.codex",
        &json!({"hooks": {"Stop": [{"hooks": [hook("echo w")]}]}}),
    );
    write_json(
        &settings(&ctx),
        &json!({"hooks": {"SessionStart": [{"hooks": [hook("echo y")]}]}}),
    );
    let db = store::db_path(&ctx);
    let db_before = std::fs::read(&db).unwrap();

    let st = status(&ctx);
    assert_eq!(std::fs::read(&db).unwrap(), db_before, "读路径推导零写");
    let claude = element(&st, "hooks:claude");
    assert_eq!(claude["state"], "managed-ok", "未生效建议不算待同步");
    let legacy = claude["observed"]["legacy"].clone();
    assert_eq!(
        commands(&legacy),
        ["echo x", "echo z"],
        "已在实际里的 y 不提示"
    );
    assert_eq!(legacy[0]["source_key"], "hooks.claude");
    let codex_legacy = element(&st, "hooks:codex")["observed"]["legacy"].clone();
    assert_eq!(commands(&codex_legacy), ["echo w"]);

    // 加入 x：出现在实际配置里，条目字段（matcher、timeout）原样带上
    let x_id = legacy[0]["id"].as_str().unwrap().to_string();
    let base = claude["observed"]["base"].as_str().unwrap().to_string();
    let env = apply(&ctx, "hooks:claude", &base, &["--legacy", &x_id]);
    assert!(env.ok, "{:?}", env.errors);
    let after = read_json(&settings(&ctx));
    assert_eq!(
        after["hooks"]["Stop"],
        json!([{"matcher": "", "hooks": [{"type": "command", "command": "echo x", "timeout": 5}]}])
    );
    assert_eq!(
        after["hooks"]["SessionStart"],
        json!([{"hooks": [hook("echo y")]}])
    );
    // 写路径清理：已在实际里的 x、y 从库里删掉，只剩 z
    let legacy = observed(&ctx, "hooks:claude")["legacy"].clone();
    assert_eq!(commands(&legacy), ["echo z"]);
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let stored: String = conn
        .query_row(
            "SELECT value_json FROM hook_sources WHERE key = 'hooks.claude'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(conn);
    assert!(
        !stored.contains("echo x") && !stored.contains("echo y"),
        "{stored}"
    );

    // 丢弃 z：库中无该来源；实际文件不动
    let file_before = std::fs::read(settings(&ctx)).unwrap();
    let z_id = legacy[0]["id"].as_str().unwrap().to_string();
    let env = run(&ctx, "hooks:discard-legacy", &["--legacy", &z_id]);
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["source_removed"], true);
    assert!(!env.lock.held, "丢弃只写库，不取 apply 锁");
    assert_eq!(legacy_rows(&ctx), ["hooks.codex"]);
    assert_eq!(std::fs::read(settings(&ctx)).unwrap(), file_before);
    assert_eq!(observed(&ctx, "hooks:claude")["legacy"], json!([]));

    // 之后用户在实际配置里删掉 x：不会再冒出来
    let base = base_of(&ctx, "hooks:claude");
    let env = apply_value(
        &ctx,
        "hooks:claude",
        &base,
        &json!({"SessionStart": [{"hooks": [hook("echo y")]}]}),
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(observed(&ctx, "hooks:claude")["legacy"], json!([]));

    // Codex 旧来源丢弃后整行删除；同一 id 再丢弃被拒
    let w_id = codex_legacy[0]["id"].as_str().unwrap().to_string();
    let env = run(&ctx, "hooks:discard-legacy", &["--legacy", &w_id]);
    assert!(env.ok, "{:?}", env.errors);
    assert!(legacy_rows(&ctx).is_empty(), "丢弃后库中无该来源");
    let again = run(&ctx, "hooks:discard-legacy", &["--legacy", &w_id]);
    assert!(!again.ok);
    assert_eq!(again.errors[0].code, "HOOK_ERROR");
    let _ = std::fs::remove_dir_all(&root);
}

/// 旧库里与内置相同的来源不产生建议；旧表损坏不挡住 hooks 页，也不回显内容。
#[test]
fn builtin_or_corrupt_legacy_rows_yield_nothing_and_never_block() {
    let (root, ctx) = temp_ctx("legacy-builtin");
    let builtin: Value =
        serde_json::from_str(include_str!("../../config/claude-hooks.json")).unwrap();
    let mut row = builtin.as_object().unwrap().clone();
    row.remove("settings_path");
    write_legacy_row(&ctx, "hooks.claude", &Value::Object(row));
    let marker = "LEGACY-MARKER-4a1e";
    write_legacy_row(&ctx, "hooks.codex", &json!({"hooks": [marker]}));
    {
        let conn = store::open_write(&ctx).unwrap();
        conn.execute(
            "INSERT INTO hook_sources VALUES ('claude', ?1, 'r', 't')",
            [format!("{{not json {marker}")],
        )
        .unwrap();
    }
    let env = run(&ctx, "hooks:status", &[]);
    assert!(env.ok, "{:?}", env.errors);
    for el in env.data["elements"].as_array().unwrap() {
        assert_eq!(el["observed"]["legacy"], json!([]), "{el}");
    }
    assert!(!serde_json::to_string(&env).unwrap().contains(marker));
    let _ = std::fs::remove_dir_all(&root);
}

/// hooks 写入不动接入/运行参数键（agents:configure 的写入基准不变）；
/// 反之 agents:configure 写入不改变 hooks（hooks 的 base 不变）。
#[test]
fn hooks_and_agent_settings_never_overwrite_each_other() {
    let (root, ctx) = temp_ctx("cross");
    std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
    let configured = {
        let status = run(&ctx, "agents:status", &[]);
        let base = status.data["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == "agents:claude-code")
            .unwrap()["observed"]["current_config"]["base"]
            .as_str()
            .unwrap()
            .to_string();
        run(
            &ctx,
            "agents:configure",
            &[
                "--only",
                "agents:claude-code",
                "--base",
                &base,
                "--runtime",
                r#"{"claude_effort_level": "high", "automation_mode": "auto"}"#,
            ],
        )
    };
    assert!(configured.ok, "{:?}", configured.errors);
    let agent_base = |ctx: &Context| -> String {
        let status = run(ctx, "agents:status", &[]);
        status.data["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == "agents:claude-code")
            .unwrap()["observed"]["current_config"]["base"]
            .as_str()
            .unwrap()
            .to_string()
    };

    // hooks 写入：接入/运行参数键语义不变，agents 的写入基准不变
    let before = read_json(&settings(&ctx));
    let agents_base_before = agent_base(&ctx);
    let hooks = json!({"Stop": [{"hooks": [hook("echo done")]}]});
    let env = apply_value(&ctx, "hooks:claude", &base_of(&ctx, "hooks:claude"), &hooks);
    assert!(env.ok, "{:?}", env.errors);
    let after = read_json(&settings(&ctx));
    assert_eq!(after["hooks"], hooks);
    assert_eq!(without_hooks(&after), without_hooks(&before));
    assert_eq!(agent_base(&ctx), agents_base_before);

    // agents:configure 写入：hooks 子树与 hooks 的 base 不变
    let hooks_base = base_of(&ctx, "hooks:claude");
    let env = run(
        &ctx,
        "agents:configure",
        &[
            "--only",
            "agents:claude-code",
            "--base",
            &agents_base_before,
            "--runtime",
            r#"{"claude_effort_level": "low"}"#,
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    let after_configure = read_json(&settings(&ctx));
    assert_eq!(after_configure["effortLevel"], "low");
    assert_eq!(after_configure["hooks"], hooks);
    assert_eq!(base_of(&ctx, "hooks:claude"), hooks_base);
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

/// 实例的 hooks 文件是 hooks 目标；profiles:apply 不再渲染 hooks，前后实例 hooks 字节不变。
#[test]
fn instance_hooks_are_targets_and_profiles_apply_leaves_them_alone() {
    let (root, ctx) = temp_ctx("profiles");
    save_profile(
        &ctx,
        "cc-ly",
        r#"{"display_name":"Claude ly","bin":"claude","profile":{"base_agent":"claude-code","config_dir":"~/.claude-ly","model":"claude-opus-5"},"_replace":true}"#,
    );
    save_profile(
        &ctx,
        "cx-work",
        r#"{"display_name":"Codex 工作","bin":"codex","profile":{"base_agent":"codex","config_dir":"~/.codex-work"},"_replace":true}"#,
    );
    // 目录还不存在：可见但不能写
    let st = status(&ctx);
    let cc = element(&st, "hooks:profile:cc-ly");
    assert_eq!(cc["spec"]["target"], "~/.claude-ly/settings.json");
    assert_eq!(cc["state"], "missing");
    assert!(cc["observed"]["blocked_reason"].as_str().is_some());
    assert_eq!(cc["actions"], json!([]));
    assert_eq!(
        element(&st, "hooks:profile:cx-work")["spec"]["target"],
        "~/.codex-work/hooks.json"
    );

    // profiles:apply 建目录，但不生成 hooks
    for id in ["profiles:cc-ly", "profiles:cx-work"] {
        let env = apply_profile(&ctx, id);
        assert!(env.ok, "{:?}", env.errors);
    }
    let cc_settings = ctx.home_dir.join(".claude-ly/settings.json");
    let cx_hooks = ctx.home_dir.join(".codex-work/hooks.json");
    assert!(
        read_json(&cc_settings).get("hooks").is_none(),
        "实例 settings.json 不再带 hooks"
    );
    assert!(!cx_hooks.exists(), "实例不再生成 hooks.json");

    // 在 hooks 页给两个实例写 hooks
    let cc_hooks = json!({"Stop": [{"hooks": [hook("echo cc")]}]});
    let env = apply_value(
        &ctx,
        "hooks:profile:cc-ly",
        &base_of(&ctx, "hooks:profile:cc-ly"),
        &cc_hooks,
    );
    assert!(env.ok, "{:?}", env.errors);
    let cx = json!({"Stop": [{"hooks": [hook("echo cx")]}]});
    let env = apply_value(&ctx, "hooks:profile:cx-work", "missing", &cx);
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        read_json(&cc_settings)["model"],
        "claude-opus-5",
        "实例自己的键保留"
    );

    // 再次 profiles:apply：实例 hooks 字节不变（Codex 文件、Claude hooks 子树）
    let cc_base = base_of(&ctx, "hooks:profile:cc-ly");
    let cx_bytes = std::fs::read(&cx_hooks).unwrap();
    for id in ["profiles:cc-ly", "profiles:cx-work"] {
        let env = apply_profile(&ctx, id);
        assert!(
            env.ok,
            "改过实例 hooks 不算实例配置被外部改过: {:?}",
            env.errors
        );
    }
    assert_eq!(std::fs::read(&cx_hooks).unwrap(), cx_bytes);
    assert_eq!(base_of(&ctx, "hooks:profile:cc-ly"), cc_base);
    assert_eq!(read_json(&cc_settings)["hooks"], cc_hooks);
    let profiles = run(&ctx, "profiles:status", &[]);
    for el in profiles.data["elements"].as_array().unwrap() {
        assert_eq!(el["state"], "managed-ok", "{el}");
    }
    // 实例目标不带旧版建议，只有主助手才对照旧来源
    assert_eq!(observed(&ctx, "hooks:profile:cc-ly")["legacy"], json!([]));
    let _ = std::fs::remove_dir_all(&root);
}

/// 旧版本 profiles 记的写入指纹含 hooks：升级后在 hooks 页改实例 hooks，再应用实例不误报。
#[test]
fn old_profile_fingerprint_with_hooks_does_not_block_after_hooks_edit() {
    let (root, ctx) = temp_ctx("profile-legacy-fp");
    save_profile(
        &ctx,
        "cc-old",
        r#"{"display_name":"Claude old","bin":"claude","profile":{"base_agent":"claude-code","config_dir":"~/.claude-old","model":"m1"},"_replace":true}"#,
    );
    // 模拟旧版本写出的实例文件与写入指纹（env/model/hooks）
    let path = ctx.home_dir.join(".claude-old/settings.json");
    let old_file = json!({"env": {}, "model": "m1", "hooks": {"Stop": [{"hooks": [hook("old")]}]}});
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // 旧版本写盘格式：pretty JSON + 换行
    std::fs::write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(&old_file).unwrap()),
    )
    .unwrap();
    let fp = sha(&serde_json::to_vec(
        &json!({"env": {}, "model": "m1", "hooks": old_file["hooks"]}),
    )
    .unwrap());
    {
        let conn = store::open_write(&ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS profile_bindings (
                profile_id TEXT NOT NULL, file TEXT NOT NULL, applied_digest TEXT NOT NULL,
                updated_at TEXT NOT NULL, PRIMARY KEY (profile_id, file))",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO profile_bindings VALUES ('cc-old', 'settings.json', ?1, 't')",
            [&fp],
        )
        .unwrap();
    }
    let st = run(&ctx, "profiles:status", &[]);
    assert_eq!(
        st.data["elements"][0]["state"], "managed-ok",
        "旧指纹照常认"
    );

    let base = base_of(&ctx, "hooks:profile:cc-old");
    let env = apply_value(
        &ctx,
        "hooks:profile:cc-old",
        &base,
        &json!({"Stop": [{"hooks": [hook("new")]}]}),
    );
    assert!(env.ok, "{:?}", env.errors);
    let env = apply_profile(&ctx, "profiles:cc-old");
    assert!(env.ok, "只改了 hooks，实例写入照常: {:?}", env.errors);
    assert_eq!(
        read_json(&path)["hooks"]["Stop"][0]["hooks"][0]["command"],
        "new"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// status 在已有库、旧来源与实际文件都在时同样零写。
#[test]
fn status_is_zero_write_on_existing_db_and_files() {
    let (root, ctx) = temp_ctx("zero-write");
    write_legacy_row(
        &ctx,
        "hooks.claude",
        &json!({"hooks": {"Stop": [{"hooks": [hook("x")]}]}}),
    );
    write_json(&settings(&ctx), &json!({"model": "m"}));
    let db = store::db_path(&ctx);
    let db_before = std::fs::read(&db).unwrap();
    let file_before = std::fs::read(settings(&ctx)).unwrap();
    for _ in 0..2 {
        let st = status(&ctx);
        assert_eq!(
            commands(&element(&st, "hooks:claude")["observed"]["legacy"]),
            ["x"]
        );
    }
    assert_eq!(std::fs::read(&db).unwrap(), db_before);
    assert_eq!(std::fs::read(settings(&ctx)).unwrap(), file_before);
    assert!(!ctx.home_dir.join(".codex").exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// 空 hooks：Claude 移除 hooks 键；不存在的文件不会因为空 hooks 被建出来。
#[test]
fn empty_hooks_remove_claude_key_and_never_create_empty_files() {
    let (root, ctx) = temp_ctx("empty");
    let env = apply_value(&ctx, "hooks:codex", "missing", &json!({}));
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["skipped"], json!(["hooks:codex"]));
    assert!(!codex_hooks(&ctx).exists());

    write_json(
        &settings(&ctx),
        &json!({"model": "m", "hooks": {"Stop": [{"hooks": [hook("x")]}]}}),
    );
    let env = apply_value(
        &ctx,
        "hooks:claude",
        &base_of(&ctx, "hooks:claude"),
        &json!({}),
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(read_json(&settings(&ctx)), json!({"model": "m"}));
    assert_eq!(base_of(&ctx, "hooks:claude"), "missing");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn invalid_values_and_args_are_rejected_without_writes_or_echo() {
    let (root, ctx) = temp_ctx("invalid");
    let marker = "HOOKS-ARG-MARKER-e81c";
    write_json(&settings(&ctx), &json!({"model": "m"}));
    let before = std::fs::read(settings(&ctx)).unwrap();
    let bad_values = [
        "{not json".to_string(),
        "[]".to_string(),
        json!({"Stop": []}).to_string(),
        json!({"Stop": [{}]}).to_string(),
        json!({"Stop": [{"hooks": [marker]}]}).to_string(),
        json!({"Stop": [{"hooks": [{"type": "command", "command": " "}]}]}).to_string(),
        json!({"Stop": [{"matcher": 1, "hooks": [hook(marker)]}]}).to_string(),
        json!({"Stop": [{"hooks": [hook("a\0b")]}]}).to_string(),
        json!({"": [{"hooks": [hook(marker)]}]}).to_string(),
    ];
    for value in &bad_values {
        let env = run(
            &ctx,
            "hooks:apply",
            &[
                "--only",
                "hooks:claude",
                "--base",
                "missing",
                "--value",
                value,
            ],
        );
        assert!(!env.ok, "{value}");
        assert_eq!(env.errors[0].code, "HOOK_ERROR");
        assert!(!env.lock.held, "预检拒绝不取锁: {value}");
        assert!(
            !serde_json::to_string(&env).unwrap().contains(marker),
            "{value}"
        );
    }
    for args in [
        vec![],
        vec!["--dry-run"],
        vec!["--only", "hooks:claude", "--allow-takeover"],
        vec!["--only", "hooks:claude", "--base", "missing"],
        vec!["--only", marker, "--base", "missing", "--value", "{}"],
        vec![
            "--only",
            "hooks:claude",
            "--base",
            "missing",
            "--value",
            "{}",
            marker,
        ],
        vec![
            "--only",
            "hooks:profile:nobody",
            "--base",
            "missing",
            "--value",
            "{}",
        ],
        vec![
            "--only",
            "hooks:claude",
            "--base",
            "missing",
            "--legacy",
            "legacy-0123456789abcdef",
        ],
        vec![
            "--only",
            "hooks:claude",
            "--base",
            "missing",
            "--cleanup",
            "cleanup-0123456789abcdef",
        ],
    ] {
        let env = run(&ctx, "hooks:apply", &args);
        assert!(!env.ok, "{args:?}");
        assert_eq!(env.errors[0].code, "HOOK_ERROR", "{args:?}");
        assert!(!env.lock.held, "{args:?}");
        assert!(
            !serde_json::to_string(&env).unwrap().contains(marker),
            "{args:?}"
        );
    }
    for args in [
        vec!["--only", "hooks:claude"],
        vec!["--dry-run"],
        vec!["x"],
        vec!["--offline", "--offline"],
    ] {
        let env = run(&ctx, "hooks:status", &args);
        assert!(!env.ok, "{args:?}");
        assert_eq!(env.errors[0].code, "HOOK_ERROR");
    }
    for args in [
        vec![],
        vec!["--legacy", "legacy-0123456789abcdef"],
        vec!["--legacy", marker],
    ] {
        let env = run(&ctx, "hooks:discard-legacy", &args);
        assert!(!env.ok, "{args:?}");
        assert!(
            !serde_json::to_string(&env).unwrap().contains(marker),
            "{args:?}"
        );
    }
    assert_eq!(std::fs::read(settings(&ctx)).unwrap(), before);
    assert!(!store::db_path(&ctx).exists(), "拒绝路径不建库");
    assert!(!ctx.data_dir.join("agent-manager.apply.lock").exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// 断链、指向目录的链接与坏 JSON：只报原因，不提供写入，文件不变。
#[test]
fn broken_or_directory_links_and_bad_json_are_blocked_without_writes() {
    let (root, ctx) = temp_ctx("blocked");
    std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    std::fs::write(codex_hooks(&ctx), "{bad json").unwrap();
    let mut blocked = vec!["hooks:codex"];
    #[cfg(unix)]
    {
        std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
        std::os::unix::fs::symlink(root.join("gone.json"), settings(&ctx)).unwrap();
        blocked.push("hooks:claude");
        save_profile(
            &ctx,
            "cc-dir",
            r#"{"display_name":"Claude dir","bin":"claude","profile":{"base_agent":"claude-code","config_dir":"~/.claude-dir"},"_replace":true}"#,
        );
        let elsewhere = root.join("a-directory");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::create_dir_all(ctx.home_dir.join(".claude-dir")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, ctx.home_dir.join(".claude-dir/settings.json"))
            .unwrap();
        blocked.push("hooks:profile:cc-dir");
    }
    let st = status(&ctx);
    for id in blocked {
        let el = element(&st, id);
        assert_eq!(el["state"], "unmanaged-detected", "{id}");
        assert_eq!(el["observed"]["base"], Value::Null, "{id}");
        assert_eq!(el["observed"]["hooks"], Value::Null, "{id}");
        assert!(el["observed"]["blocked_reason"].as_str().is_some(), "{id}");
        assert_eq!(el["actions"], json!([]), "{id}");
        let env = apply_value(
            &ctx,
            id,
            "missing",
            &json!({"Stop": [{"hooks": [hook("x")]}]}),
        );
        assert!(!env.ok, "{id}");
    }
    assert_eq!(
        std::fs::read_to_string(codex_hooks(&ctx)).unwrap(),
        "{bad json"
    );
    #[cfg(unix)]
    {
        assert!(!root.join("gone.json").exists(), "断链不会被写出目标文件");
        assert!(std::fs::symlink_metadata(settings(&ctx))
            .unwrap()
            .file_type()
            .is_symlink());
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// settings.json / hooks.json 是指向普通文件的符号链接（dotfiles 仓库常见）：与
/// agents:configure 同一裁决，解析到最终目标读写；CAS 比对目标字节，链接本身不变。
#[cfg(unix)]
#[test]
fn symlinked_targets_are_read_and_written_through_to_the_real_file() {
    let (root, ctx) = temp_ctx("symlink");
    let dotfiles = root.join("dotfiles");
    let real_settings = dotfiles.join("claude-settings.json");
    write_json(
        &real_settings,
        &json!({"model": "m", "hooks": {"Stop": [{"hooks": [hook("echo real")]}]}}),
    );
    std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
    std::os::unix::fs::symlink(&real_settings, settings(&ctx)).unwrap();
    // Codex：整个 ~/.codex 目录链接到 dotfiles
    let real_codex_dir = dotfiles.join("codex");
    write_json(
        &real_codex_dir.join("hooks.json"),
        &json!({"hooks": {"SessionStart": [{"hooks": [hook("echo codex")]}]}}),
    );
    std::os::unix::fs::symlink(&real_codex_dir, ctx.home_dir.join(".codex")).unwrap();

    let st = status(&ctx);
    let claude = element(&st, "hooks:claude");
    assert_eq!(claude["state"], "managed-ok");
    assert_eq!(
        claude["observed"]["hooks"]["Stop"][0]["hooks"][0]["command"],
        "echo real"
    );
    let codex = element(&st, "hooks:codex");
    assert_eq!(codex["state"], "managed-ok");
    assert_eq!(
        codex["observed"]["base"],
        sha(&std::fs::read(real_codex_dir.join("hooks.json")).unwrap())
    );

    // 写入：落到最终目标，链接不变，其他键保留
    let base = claude["observed"]["base"].as_str().unwrap().to_string();
    let next = json!({"Stop": [{"hooks": [hook("echo edited")]}]});
    let env = apply_value(&ctx, "hooks:claude", &base, &next);
    assert!(env.ok, "{:?}", env.errors);
    assert!(std::fs::symlink_metadata(settings(&ctx))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read_link(settings(&ctx)).unwrap(), real_settings);
    let written = read_json(&real_settings);
    assert_eq!(written["hooks"], next);
    assert_eq!(written["model"], "m");
    let codex_base = codex["observed"]["base"].as_str().unwrap().to_string();
    let env = apply_value(&ctx, "hooks:codex", &codex_base, &json!({}));
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        read_json(&real_codex_dir.join("hooks.json")),
        json!({"hooks": {}})
    );
    assert!(std::fs::symlink_metadata(ctx.home_dir.join(".codex"))
        .unwrap()
        .file_type()
        .is_symlink());

    // CAS 比对的是目标字节：外部改了目标文件，旧 base 被拒
    let stale = base_of(&ctx, "hooks:claude");
    write_json(
        &real_settings,
        &json!({"model": "m", "hooks": {"Stop": [{"hooks": [hook("changed outside")]}]}}),
    );
    let before = std::fs::read(&real_settings).unwrap();
    let refused = apply_value(&ctx, "hooks:claude", &stale, &json!({}));
    assert!(!refused.ok);
    assert!(refused.errors[0].message.contains("已在外部修改"));
    assert_eq!(std::fs::read(&real_settings).unwrap(), before);
    let _ = std::fs::remove_dir_all(&root);
}

/// 官方 hook 类型不止 command（http / mcp_tool / prompt / agent，以及将来的未知类型）：
/// 不阻塞目标，status 如实展示；编辑其中一条 command 后，其余类型逐字段保留；
/// 清理建议只看 command 类型。
#[test]
fn non_command_hook_types_are_shown_and_preserved_on_edit() {
    let (root, ctx) = temp_ctx("hook-types");
    let others = json!([
        {"type": "http", "url": "https://hooks.example.com/pre", "headers": {"X-Token": "${HOOK_TOKEN}"}, "timeout": 10},
        {"type": "prompt", "prompt": "Is this safe? $ARGUMENTS", "model": "claude-haiku"},
        {"type": "agent", "prompt": "Review the diff", "timeout": 120},
        {"type": "mcp_tool", "server": "guard", "tool": "check"},
        {"type": "brand-new-kind", "nested": {"a": [1, 2, 3]}},
        {"type": "http", "url": "https://x", "command": "sh /old/agent-rule/hooks/block-dist-edits.sh"}
    ]);
    let mut entry_hooks = others.as_array().unwrap().clone();
    entry_hooks.insert(1, hook("echo mine"));
    let file = json!({
        "model": "m",
        "hooks": {
            "PreToolUse": [{"matcher": "Edit|MultiEdit|Write", "hooks": entry_hooks}],
            "Stop": [{"hooks": [{"type": "prompt", "prompt": "done?"}]}]
        }
    });
    write_json(&settings(&ctx), &file);

    let claude = element(&status(&ctx), "hooks:claude");
    assert_eq!(claude["state"], "managed-ok", "非 command 类型不阻塞目标");
    assert_eq!(claude["observed"]["hooks"], file["hooks"]);
    assert_eq!(claude["observed"]["hook_count"], 8);
    assert_eq!(
        claude["observed"]["cleanup"],
        json!([]),
        "http 类型即使带 command 字段也不算清理建议"
    );

    // 与界面一致：拿实际 hooks，只改 command 那一条，其余原样带回
    let mut edited = claude["observed"]["hooks"].clone();
    edited["PreToolUse"][0]["hooks"][1]["command"] = json!("echo mine v2");
    let base = claude["observed"]["base"].as_str().unwrap().to_string();
    let env = apply_value(&ctx, "hooks:claude", &base, &edited);
    assert!(env.ok, "{:?}", env.errors);
    let after = read_json(&settings(&ctx));
    let list = after["hooks"]["PreToolUse"][0]["hooks"].as_array().unwrap();
    assert_eq!(list[1]["command"], "echo mine v2");
    let kept: Vec<Value> = list
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 1)
        .map(|(_, v)| v.clone())
        .collect();
    assert_eq!(Value::Array(kept), others, "其余类型逐字段保留");
    assert_eq!(after["hooks"]["Stop"], file["hooks"]["Stop"]);
    assert_eq!(after["model"], "m");

    // 原文编辑可直接改非 command 类型；只做骨架校验
    let base = base_of(&ctx, "hooks:claude");
    let mut raw = after["hooks"].clone();
    raw["PreToolUse"][0]["hooks"][0]["url"] = json!("https://hooks.example.com/v2");
    let env = apply_value(&ctx, "hooks:claude", &base, &raw);
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        read_json(&settings(&ctx))["hooks"]["PreToolUse"][0]["hooks"][0]["url"],
        "https://hooks.example.com/v2"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn disabled_agent_target_is_never_probed_or_written() {
    let (root, ctx) = temp_ctx("disabled");
    let mut policy = store::builtin_policy();
    policy["agents"]["codex"]["enabled"] = Value::Bool(false);
    {
        let conn = store::open_write(&ctx).unwrap();
        store::write_policy(&conn, &policy).unwrap();
    }
    std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    std::fs::write(codex_hooks(&ctx), "{bad json").unwrap();
    let codex = element(&status(&ctx), "hooks:codex");
    assert_eq!(codex["state"], "disabled");
    assert_eq!(codex["observed"]["target_exists"], Value::Null);
    assert_eq!(codex["actions"], json!([]));
    let env = apply_value(&ctx, "hooks:codex", "missing", &json!({}));
    assert!(!env.ok);
    assert!(env.errors[0].message.contains("已停用"));
    let _ = std::fs::remove_dir_all(&root);
}

/// 旧 hook_target_bindings 不再是写入门槛：记录对不上实际文件也照常编辑。
#[test]
fn old_bindings_table_is_no_longer_a_write_gate() {
    let (root, ctx) = temp_ctx("old-bindings");
    {
        let conn = store::open_write(&ctx).unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE hook_target_bindings (
                target_id TEXT PRIMARY KEY NOT NULL, applied_digest TEXT NOT NULL,
                source_revision TEXT NOT NULL, updated_at TEXT NOT NULL);
             INSERT INTO hook_target_bindings VALUES ('hooks:codex', '{0}', '{0}', 't');",
            "0".repeat(64)
        ))
        .unwrap();
    }
    write_json(
        &codex_hooks(&ctx),
        &json!({"hooks": {"Stop": [{"hooks": [hook("mine")]}]}}),
    );
    let codex = element(&status(&ctx), "hooks:codex");
    assert_eq!(codex["state"], "managed-ok");
    let base = codex["observed"]["base"].as_str().unwrap().to_string();
    let env = apply_value(
        &ctx,
        "hooks:codex",
        &base,
        &json!({"Stop": [{"hooks": [hook("mine2")]}]}),
    );
    assert!(env.ok, "{:?}", env.errors);
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
        apply_value(
            &worker_ctx,
            "hooks:codex",
            "missing",
            &json!({"Stop": [{"hooks": [hook("x")]}]}),
        )
    })
    .join()
    .unwrap();
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "HOOK_ERROR");
    assert!(!ctx.home_dir.exists(), "锁竞争失败不得写目标");
    let _ = std::fs::remove_dir_all(&root);
}
