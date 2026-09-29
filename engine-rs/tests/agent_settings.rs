//! 助手运行参数落到助手自己的配置文件：实际文件为准。status 给出实际值、写入基准与当前接入；
//! 写入只动本次涉及的那组键，基准对不上就拒绝，切换前把还没保存的实际接入回填进库，
//! YOLO 少了显式确认标志一律拒绝。

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use agent_manager_engine::{execute, store, Context, Envelope, Request};

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    let base = std::env::temp_dir().join(format!(
        "am-agent-settings-{}-{}-{:x}",
        std::process::id(),
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&base);
    let ctx = Context {
        data_dir: base.join("data"),
        home_dir: base.join("home"),
    };
    fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
    (base, ctx)
}

fn request(command: &str, args: &[&str]) -> Request {
    Request {
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        run_id: "cfg-test".to_string(),
    }
}

fn configure_secret_source(ctx: &Context) -> PathBuf {
    let path = ctx.home_dir.join("secrets.env");
    fs::write(&path, "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let configured = execute(
        ctx,
        &request("secrets:source", &["--path", path.to_str().unwrap()]),
    );
    assert!(configured.ok, "{:?}", configured.errors);
    path
}

/// 写一条只有单套接入配置的定义：大多数用例只关心那一套。
fn define(ctx: &Context, profile: Value) {
    define_settings(ctx, json!({"active": "p", "profiles": {"p": profile}}));
}

/// 写一条带完整 settings 段的助手定义（内置 claude-code 的覆盖）。
fn define_settings(ctx: &Context, settings: Value) {
    // 与界面保存定义同一条路径：整体替换，否则清空某项会被深度合并吃掉
    let definition = json!({
        "_replace": true,
        "display_name": "Claude Code",
        "bin": "claude",
        "preferred_provider": "native",
        "providers": {"native": {"install": "true", "upgrade": "true", "managed": true}},
        "settings": settings,
    });
    let env = execute(
        ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.claude-code",
                "--value",
                &definition.to_string(),
            ],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
}

fn settings_file(ctx: &Context) -> Value {
    let raw = fs::read(ctx.home_dir.join(".claude/settings.json")).unwrap();
    serde_json::from_slice(&raw).unwrap()
}

fn write_settings_file(ctx: &Context, value: &Value) {
    fs::write(
        ctx.home_dir.join(".claude/settings.json"),
        serde_json::to_vec_pretty(value).unwrap(),
    )
    .unwrap();
}

/// status 里某个助手的 `observed.current_config`。
fn current_config(ctx: &Context, agent: &str) -> Value {
    let status = execute(ctx, &request("agents:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == format!("agents:{agent}"))
        .unwrap()["observed"]["current_config"]
        .clone()
}

/// status 给出的写入基准；没有时用一个占位值，让参数校验之后的拒绝路径照样可测。
fn base_of(ctx: &Context, agent: &str) -> String {
    current_config(ctx, agent)["base"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| "0".repeat(64))
}

/// 与界面同一条路径：先读 status 拿 base，再带着它写。
fn configure(ctx: &Context, agent: &str, args: &[&str]) -> Envelope {
    let base = base_of(ctx, agent);
    configure_with_base(ctx, agent, &base, args)
}

fn configure_with_base(ctx: &Context, agent: &str, base: &str, args: &[&str]) -> Envelope {
    let only = format!("agents:{agent}");
    let mut full = vec!["--only", only.as_str(), "--base", base];
    full.extend_from_slice(args);
    execute(ctx, &request("agents:configure", &full))
}

fn preview_runtime(ctx: &Context, agent: &str, base: &str, patch: &str) -> Envelope {
    let only = format!("agents:{agent}");
    execute(
        ctx,
        &request(
            "agents:preview-runtime",
            &["--only", &only, "--base", base, "--runtime", patch],
        ),
    )
}

fn settings_in_db(ctx: &Context, agent: &str) -> Value {
    let policy = execute(ctx, &request("policy:get", &[]));
    assert!(policy.ok, "{:?}", policy.errors);
    policy.data["effective"]["agents"]["definitions"][agent]["settings"].clone()
}

/// 数据目录下全部文件的字节（库文件、锁文件等），用于「拒绝时库不变」与「库里没有明文」。
fn data_dir_bytes(ctx: &Context) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(&ctx.data_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                out.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

fn db_bytes(ctx: &Context) -> Option<Vec<u8>> {
    fs::read(store::db_path(ctx)).ok()
}

fn file_bytes(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap()
}

#[test]
fn 写入只动自己那几条路径_别人的键原样保留() {
    let (base, ctx) = temp_ctx("merge");
    // 用户自己的设置先在那儿：hooks 归 hooks 域，statusLine 和别的 env 是用户的
    write_settings_file(
        &ctx,
        &json!({
            "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "x"}]}]},
            "statusLine": {"type": "command", "command": "/x/statusline.sh"},
            "env": {"MY_OWN": "keep-me"}
        }),
    );
    let env = configure(
        &ctx,
        "claude-code",
        &["--runtime", r#"{"claude_effort_level": "high"}"#],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.lock.held, "写动作要取排他锁");

    let after = settings_file(&ctx);
    assert!(
        after["hooks"]["SessionStart"].is_array(),
        "hooks 域的内容不能动"
    );
    assert_eq!(after["statusLine"]["command"], "/x/statusline.sh");
    assert_eq!(after["env"]["MY_OWN"], "keep-me");
    assert_eq!(after["effortLevel"], "high");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 运行参数完整预览与实际写入同字节_且预览零写() {
    use sha2::{Digest, Sha256};
    let (base, ctx) = temp_ctx("runtime-preview-claude");
    let path = ctx.home_dir.join(".claude/settings.json");
    write_settings_file(
        &ctx,
        &json!({"env": {"MY_OWN": "keep"}, "hooks": {"SessionStart": []}}),
    );
    let old_file = file_bytes(&path);
    let old_data = data_dir_bytes(&ctx);
    let status_base = base_of(&ctx, "claude-code");
    let patch = r#"{"claude_effort_level":"high"}"#;
    let preview = preview_runtime(&ctx, "claude-code", &status_base, patch);
    assert!(preview.ok, "{:?}", preview.errors);
    let content = preview.data["content"].as_str().unwrap();
    let file_base = preview.data["file_base"].as_str().unwrap();
    assert_eq!(preview.data["path"], path.display().to_string());
    assert_eq!(preview.data["exists"], true);
    assert_eq!(file_base, format!("{:x}", Sha256::digest(&old_file)));
    assert_eq!(
        serde_json::from_str::<Value>(content).unwrap()["env"]["MY_OWN"],
        "keep"
    );
    assert_eq!(
        serde_json::from_str::<Value>(content).unwrap()["effortLevel"],
        "high"
    );
    assert_eq!(file_bytes(&path), old_file);
    assert_eq!(data_dir_bytes(&ctx), old_data);

    let old_settings = settings_in_db(&ctx, "claude-code");
    let mut changed: Value = serde_json::from_slice(&old_file).unwrap();
    changed["hooks"]["SessionStart"] = json!([{"hooks": []}]);
    write_settings_file(&ctx, &changed);
    assert_eq!(base_of(&ctx, "claude-code"), status_base);
    let changed_file = file_bytes(&path);
    let stale = configure_with_base(
        &ctx,
        "claude-code",
        &status_base,
        &["--runtime", patch, "--preview-file-base", file_base],
    );
    assert!(!stale.ok);
    assert_eq!(stale.errors[0].code, "AGENT_CONFIG_CHANGED");
    assert_eq!(file_bytes(&path), changed_file);
    assert_eq!(settings_in_db(&ctx, "claude-code"), old_settings);
    fs::write(&path, &old_file).unwrap();

    let applied = configure_with_base(
        &ctx,
        "claude-code",
        &status_base,
        &["--runtime", patch, "--preview-file-base", file_base],
    );
    assert!(applied.ok, "{:?}", applied.errors);
    assert_eq!(file_bytes(&path), content.as_bytes());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn codex_完整预览保留用户内容_外部改动后拒绝旧基准() {
    use sha2::{Digest, Sha256};
    let (base, ctx) = temp_ctx("runtime-preview-codex");
    define_codex(&ctx, json!({}));
    let dir = ctx.home_dir.join(".codex");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    fs::write(&path, "# 自己的注释\nuser_instructions = \"keep\"\n").unwrap();
    let status_base = base_of(&ctx, "codex");
    let old_file = file_bytes(&path);
    let old_data = data_dir_bytes(&ctx);
    let patch = r#"{"codex_reasoning_effort":"high"}"#;
    let preview = preview_runtime(&ctx, "codex", &status_base, patch);
    assert!(preview.ok, "{:?}", preview.errors);
    let content = preview.data["content"].as_str().unwrap();
    let file_base = preview.data["file_base"].as_str().unwrap();
    assert_eq!(file_base, format!("{:x}", Sha256::digest(&old_file)));
    assert!(content.contains("# 自己的注释"));
    assert!(content.contains("user_instructions = \"keep\""));
    assert!(content.contains("model_reasoning_effort = \"high\""));
    assert_eq!(file_bytes(&path), old_file);
    assert_eq!(data_dir_bytes(&ctx), old_data);

    let applied = configure_with_base(
        &ctx,
        "codex",
        &status_base,
        &["--runtime", patch, "--preview-file-base", file_base],
    );
    assert!(applied.ok, "{:?}", applied.errors);
    assert_eq!(file_bytes(&path), content.as_bytes());

    fs::write(
        &path,
        "model_reasoning_effort = \"low\"\nuser_instructions = \"external\"\n",
    )
    .unwrap();
    let changed = file_bytes(&path);
    let stale = preview_runtime(&ctx, "codex", &status_base, patch);
    assert!(!stale.ok);
    assert_eq!(stale.errors[0].code, "AGENT_CONFIG_CHANGED");
    assert_eq!(file_bytes(&path), changed);
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 不存在的配置文件预览不创建文件或数据库() {
    let (base, ctx) = temp_ctx("runtime-preview-missing");
    let status_base = base_of(&ctx, "claude-code");
    let preview = preview_runtime(
        &ctx,
        "claude-code",
        &status_base,
        r#"{"default_model":"claude-sonnet"}"#,
    );
    assert!(preview.ok, "{:?}", preview.errors);
    assert_eq!(preview.data["exists"], false);
    assert_eq!(preview.data["file_base"], "missing");
    assert!(!ctx.home_dir.join(".claude/settings.json").exists());
    assert!(!store::db_path(&ctx).exists());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 缺_base_或缺_runtime_时拒绝_且零写() {
    let (base, ctx) = temp_ctx("usage");
    define(&ctx, json!({"base_url": "https://api.example.com"}));
    let db_before = db_bytes(&ctx);

    for args in [
        vec![
            "--only",
            "agents:claude-code",
            "--runtime",
            r#"{"codex_web_search": "live"}"#,
        ],
        vec!["--only", "agents:claude-code", "--base", "x"],
    ] {
        let env = execute(&ctx, &request("agents:configure", &args));
        assert!(!env.ok, "{args:?}");
        assert_eq!(env.errors[0].code, "USAGE_ERROR", "{args:?}");
    }
    // --connection 是停用的写入第三方接入的选项：与 --use 同样按「主配置只放官方账号」拒绝
    let retired = execute(
        &ctx,
        &request(
            "agents:configure",
            &[
                "--only",
                "agents:claude-code",
                "--base",
                "x",
                "--save-current",
                "--connection",
                "{}",
            ],
        ),
    );
    assert_eq!(retired.errors[0].code, "AGENT_MAIN_OFFICIAL_ONLY");
    assert!(!ctx.home_dir.join(".claude/settings.json").exists());
    assert_eq!(db_bytes(&ctx), db_before);

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 状态输出实际值_写入基准与当前接入_缺库不建库且零写() {
    let (base, ctx) = temp_ctx("status-live");
    let bytes = serde_json::to_vec_pretty(&json!({"env": {
        "ANTHROPIC_BASE_URL": "https://mine.example.com",
        "ANTHROPIC_AUTH_TOKEN": "status-live-secret",
        "ANTHROPIC_MODEL": "claude-mine"
    }}))
    .unwrap();
    fs::write(ctx.home_dir.join(".claude/settings.json"), &bytes).unwrap();

    let current = current_config(&ctx, "claude-code");
    assert_eq!(current["state"], "ready");
    assert_eq!(current["config"]["base_url"], "https://mine.example.com");
    assert_eq!(current["config"]["credential"]["configured"], true);
    assert_eq!(current["config"]["credential"]["saved_var"], Value::Null);
    let digest = current["base"].as_str().unwrap();
    assert_eq!(digest.len(), 64);
    // 库里什么都没有：不把任何内置默认当作当前，实际配置就是未保存的当前项
    assert_eq!(current["current"]["profile"], Value::Null);
    assert_eq!(current["current"]["default"], false);
    assert_eq!(current["current"]["last_written"], Value::Null);
    assert!(!current.to_string().contains("status-live-secret"));
    // 没有配置文件的 Codex：默认账号，也有基准可写
    let codex = current_config(&ctx, "codex");
    assert_eq!(codex["state"], "missing");
    assert_eq!(codex["current"]["default"], true);
    assert!(codex["base"].is_string());

    assert!(
        !ctx.data_dir.exists(),
        "只读 status 不得创建数据目录或数据库"
    );
    assert_eq!(
        file_bytes(&ctx.home_dir.join(".claude/settings.json")),
        bytes
    );
    let _ = fs::remove_dir_all(&base);
}

const LIVE_TOKEN: &str = "sk-live-token-never-in-db";

fn seed_live_token(ctx: &Context) {
    write_settings_file(
        ctx,
        &json!({"env": {
            "ANTHROPIC_BASE_URL": "https://hand.example.com",
            "ANTHROPIC_AUTH_TOKEN": LIVE_TOKEN
        }}),
    );
    define(ctx, json!({"base_url": "https://api.example.com"}));
}

#[test]
fn 外部把默认模式改为_accept_edits_如实显示_旧基准被拒_新基准只改提交项() {
    let (base, ctx) = temp_ctx("runtime-cas");
    let first = configure(
        &ctx,
        "claude-code",
        &["--runtime", r#"{"claude_effort_level": "high"}"#],
    );
    assert!(first.ok, "{:?}", first.errors);
    let stale = base_of(&ctx, "claude-code");

    let mut current = settings_file(&ctx);
    current["permissions"] = json!({"defaultMode": "acceptEdits"});
    write_settings_file(&ctx, &current);
    let edited = file_bytes(&ctx.home_dir.join(".claude/settings.json"));

    // 表单预填来源：实际值原样报出来
    let runtime = current_config(&ctx, "claude-code")["config"]["runtime"].clone();
    assert_eq!(runtime["automation_mode"], "custom");
    assert_eq!(
        runtime["automation_raw"]["permissions.defaultMode"],
        "acceptEdits"
    );
    assert_eq!(runtime["claude_effort_level"], "high");

    let db_before = db_bytes(&ctx);
    let refused = configure_with_base(
        &ctx,
        "claude-code",
        &stale,
        &["--runtime", r#"{"claude_effort_level": "low"}"#],
    );
    assert!(!refused.ok);
    assert_eq!(refused.errors[0].code, "AGENT_CONFIG_CHANGED");
    assert_eq!(
        file_bytes(&ctx.home_dir.join(".claude/settings.json")),
        edited
    );
    assert_eq!(db_bytes(&ctx), db_before);

    let applied = configure(
        &ctx,
        "claude-code",
        &["--runtime", r#"{"claude_effort_level": "low"}"#],
    );
    assert!(applied.ok, "{:?}", applied.errors);
    assert_eq!(applied.data["changed"], json!(["claude_effort_level"]));
    let after = settings_file(&ctx);
    assert_eq!(
        after["permissions"]["defaultMode"], "acceptEdits",
        "没提交的不动"
    );
    // 官方持久键 effortLevel（/effort 写的同一个键）；不写单次会话覆盖变量
    assert_eq!(after["effortLevel"], "low");
    assert!(after
        .get("env")
        .and_then(|env| env.get("CLAUDE_CODE_EFFORT_LEVEL"))
        .is_none());
    assert_eq!(
        settings_in_db(&ctx, "claude-code")["claude_effort_level"],
        "low"
    );

    // 拼错或跨形态的字段不能静默不生效
    let typo = configure(
        &ctx,
        "claude-code",
        &["--runtime", r#"{"codex_web_search": "live"}"#],
    );
    assert!(!typo.ok);
    assert_eq!(typo.errors[0].code, "USAGE_ERROR");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn yolo_没有显式确认标志一律拒绝() {
    let (base, ctx) = temp_ctx("yolo");
    // 旧版本的 yolo 开关：写新值时一并撤掉，避免与 automation_mode 冲突
    define_settings(&ctx, json!({"yolo": true}));

    let refused = configure(
        &ctx,
        "claude-code",
        &["--runtime", r#"{"automation_mode": "full"}"#],
    );
    assert!(!refused.ok, "降安全等级必须显式确认");
    assert!(
        !ctx.home_dir.join(".claude/settings.json").exists(),
        "拒绝时不得落地任何内容"
    );

    let confirmed = configure(
        &ctx,
        "claude-code",
        &[
            "--runtime",
            r#"{"automation_mode": "full"}"#,
            "--allow-yolo",
        ],
    );
    assert!(confirmed.ok, "{:?}", confirmed.errors);
    assert_eq!(
        settings_file(&ctx)["permissions"]["defaultMode"],
        "bypassPermissions"
    );
    let stored = settings_in_db(&ctx, "claude-code");
    assert_eq!(stored["automation_mode"], "full");
    assert!(stored.get("yolo").is_none());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn claude_智能自动写入_auto_且不需要高风险确认() {
    let (base, ctx) = temp_ctx("claude-auto");

    let result = configure(
        &ctx,
        "claude-code",
        &["--runtime", r#"{"automation_mode": "auto"}"#],
    );
    assert!(result.ok, "{:?}", result.errors);
    assert_eq!(settings_file(&ctx)["permissions"]["defaultMode"], "auto");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 既不是_claude_也不是_codex_的助手拒绝写运行参数_且零写() {
    let (base, ctx) = temp_ctx("kind");
    // 运行参数是按形态写各自的配置文件；别家助手没有对应的落点
    let definition = json!({
        "display_name": "OpenCode",
        "bin": "opencode",
        "preferred_provider": "system",
        "providers": {"system": {"managed": false}},
        "settings": {"active": "p", "profiles": {"p": {"base_url": "https://api.example.com"}}},
    });
    let env = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.opencode",
                "--value",
                &definition.to_string(),
            ],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);

    let refused = configure(&ctx, "opencode", &["--use-default"]);
    assert!(!refused.ok);
    assert!(!ctx.home_dir.join(".claude/settings.json").exists());
    assert!(!ctx.home_dir.join(".codex/config.toml").exists());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 非法地址与变量名在写之前就被挡住() {
    let (base, ctx) = temp_ctx("validate");
    for bad in [
        json!({"base_url": "ftp://evil.example.com"}),
        json!({"token_var": "has-dash"}),
    ] {
        define(&ctx, bad);
        let refused = configure(&ctx, "claude-code", &["--use-default"]);
        assert!(!refused.ok, "非法输入不该落地");
        assert!(!ctx.home_dir.join(".claude/settings.json").exists());
    }
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 缺库时状态读取不建库_配置写入才建() {
    let (base, ctx) = temp_ctx("zero-write");
    let status = execute(&ctx, &request("agents:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    assert!(!store::db_path(&ctx).exists(), "只读路径不得建库");
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 主配置拒绝写入第三方接入_文件与库都不动() {
    let (base, ctx) = temp_ctx("main-official-only");
    define_settings(
        &ctx,
        json!({"profiles": {"relay": {"base_url": "https://relay.example.com", "model": "m"}}}),
    );
    write_settings_file(&ctx, &json!({"env": {"MY_OWN": "keep-me"}}));
    let before = file_bytes(&ctx.home_dir.join(".claude/settings.json"));
    let db_before = db_bytes(&ctx);

    for args in [
        vec!["--use", "relay"],
        vec!["--use", "nope"],
        vec![
            "--use",
            "relay",
            "--connection",
            r#"{"base_url": "https://x.example.com"}"#,
        ],
        vec!["--use-default"],
        vec!["--save-current"],
    ] {
        let refused = configure(&ctx, "claude-code", &args);
        assert!(!refused.ok, "{args:?}");
        assert_eq!(
            refused.errors[0].code, "AGENT_MAIN_OFFICIAL_ONLY",
            "{args:?}"
        );
        assert!(refused.errors[0].message.contains("新建一个账号"));
    }
    assert_eq!(
        file_bytes(&ctx.home_dir.join(".claude/settings.json")),
        before,
        "被拒时一个字节都不许动"
    );
    assert_eq!(db_bytes(&ctx), db_before);

    let _ = fs::remove_dir_all(&base);
}
#[test]
fn 内置状态栏会部署脚本并指过去_切回不碰只撤自己那条() {
    let (base, ctx) = temp_ctx("statusline");

    let env = configure(
        &ctx,
        "claude-code",
        &["--runtime", r#"{"statusline": "builtin"}"#],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["statusline"], true);

    let script = ctx.data_dir.join("statusline.sh");
    assert!(script.is_file(), "脚本要落到数据目录");
    let body = fs::read_to_string(&script).unwrap();
    assert!(body.starts_with("#!/bin/bash"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&script).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "状态栏脚本要可执行");
    }
    // command 在 shell 里执行：按单引号引用，数据目录含空格也不会被拆开
    assert_eq!(
        settings_file(&ctx)["statusLine"]["command"],
        format!("'{}'", script.display())
    );

    // 落盘是它存在的理由：像 Claude Code 那样按绝对路径跑一次，额度落在数据目录下，
    // 助手配置目录里不多出任何文件，agents:status 能读回
    #[cfg(unix)]
    {
        use std::io::Write;
        let claude_dir = ctx.home_dir.join(".claude");
        let before: Vec<_> = fs::read_dir(&claude_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        let mut child = std::process::Command::new(&script)
            .env("HOME", &ctx.home_dir)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env("AM_STATUSLINE_NOW", "1789871000")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                br#"{"rate_limits":{"five_hour":{"used_percentage":23,"resets_at":1789900000}}}"#,
            )
            .unwrap();
        assert!(child.wait().unwrap().success());

        let key = claude_dir
            .display()
            .to_string()
            .replace('%', "%25")
            .replace('/', "%2F");
        let usage_file = ctx.data_dir.join("usage").join(format!("{key}.json"));
        assert!(usage_file.is_file(), "额度要落到数据目录的 usage/ 下");
        let after: Vec<_> = fs::read_dir(&claude_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(after, before, "助手配置目录里不能多出文件");

        let status = execute(&ctx, &request("agents:status", &[]));
        assert!(status.ok, "{:?}", status.errors);
        let claude = status.data["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|element| element["id"] == "agents:claude-code")
            .unwrap();
        assert_eq!(
            claude["observed"]["usage"]["source"],
            json!(usage_file.display().to_string())
        );
        assert_eq!(
            claude["observed"]["usage"]["windows"][0]["used_percent"],
            json!(23.0)
        );
    }

    // 切回「不碰」：只撤掉我们指过去的那条
    assert!(configure(&ctx, "claude-code", &["--runtime", r#"{"statusline": ""}"#]).ok);
    assert!(settings_file(&ctx).get("statusLine").is_none());

    let _ = fs::remove_dir_all(&base);
}

/// 模拟旧版本写下的状态：settings.json 里是裸路径的状态栏，库里的摘要按旧版口径
/// （本工具拥有的切片序列化后取 sha256）计算，与旧版本写入后落库的值一致。
fn seed_legacy_statusline(ctx: &Context, extra: Value) -> String {
    use sha2::{Digest, Sha256};
    let bare = ctx.data_dir.join("statusline.sh").display().to_string();
    let statusline = json!({"type": "command", "command": bare, "padding": 0});
    let mut file = json!({"statusLine": statusline.clone()});
    for (key, value) in extra.as_object().unwrap() {
        file[key] = value.clone();
    }
    write_settings_file(ctx, &file);
    let slice = serde_json::to_vec(&json!({"statusLine": statusline})).unwrap();
    let digest: String = Sha256::digest(&slice)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let conn = rusqlite::Connection::open(store::db_path(ctx)).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_settings_bindings (
             agent_id TEXT PRIMARY KEY NOT NULL, file TEXT NOT NULL,
             applied_digest TEXT NOT NULL, updated_at TEXT NOT NULL,
             owned_providers TEXT NOT NULL DEFAULT '[]')",
    )
    .unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO agent_settings_bindings
             (agent_id, file, applied_digest, updated_at) VALUES ('claude-code', 'settings.json', ?1, 'old')",
        [&digest],
    )
    .unwrap();
    bare
}

fn claude_observed_statusline(ctx: &Context) -> Value {
    current_config(ctx, "claude-code")["config"]["runtime"]["statusline"].clone()
}

#[test]
fn 旧版裸路径状态栏_升级后报旧格式_用状态给的基准即可改成引用形式() {
    let (base, ctx) = temp_ctx("statusline-legacy");
    define_settings(&ctx, json!({"statusline": "builtin"}));
    let bare = seed_legacy_statusline(&ctx, json!({"env": {"MY_OWN": "keep-me"}}));
    let legacy_bytes = fs::read(ctx.home_dir.join(".claude/settings.json")).unwrap();

    // 只读查询：认得是我们的，但单独报旧格式，界面据此给出修复入口；查询零写
    assert_eq!(claude_observed_statusline(&ctx), json!("builtin-legacy"));
    assert_eq!(
        fs::read(ctx.home_dir.join(".claude/settings.json")).unwrap(),
        legacy_bytes
    );

    // 旧库里的摘要记录不参与判定：用 status 给的基准写即可
    let applied = configure(
        &ctx,
        "claude-code",
        &["--runtime", r#"{"statusline": "builtin"}"#],
    );
    assert!(applied.ok, "{:?}", applied.errors);
    let after = settings_file(&ctx);
    assert_eq!(after["statusLine"]["command"], format!("'{bare}'"));
    assert_eq!(after["env"]["MY_OWN"], "keep-me");
    assert_eq!(claude_observed_statusline(&ctx), json!("builtin"));

    // 「不碰」：引用形式被撤掉
    let cleared = configure(&ctx, "claude-code", &["--runtime", r#"{"statusline": ""}"#]);
    assert!(cleared.ok, "{:?}", cleared.errors);
    assert!(settings_file(&ctx).get("statusLine").is_none());

    // 「不碰」：旧裸路径同样被撤掉
    seed_legacy_statusline(&ctx, json!({}));
    let cleared = configure(&ctx, "claude-code", &["--runtime", r#"{"statusline": ""}"#]);
    assert!(cleared.ok, "{:?}", cleared.errors);
    assert!(settings_file(&ctx).get("statusLine").is_none());

    // 用户自定义的状态栏：「不碰」时原样保留
    let mut file = settings_file(&ctx);
    file["statusLine"] = json!({"type": "command", "command": "/x/mine.sh"});
    write_settings_file(&ctx, &file);
    let kept = configure(&ctx, "claude-code", &["--runtime", r#"{"statusline": ""}"#]);
    assert!(kept.ok, "{:?}", kept.errors);
    assert_eq!(settings_file(&ctx)["statusLine"]["command"], "/x/mine.sh");
    assert_eq!(claude_observed_statusline(&ctx), json!("custom"));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 用户自己的状态栏不被碰_也不算外部修改() {
    let (base, ctx) = temp_ctx("statusline-user");
    // 用户先有自己的状态栏
    write_settings_file(
        &ctx,
        &json!({"statusLine": {"type": "command", "command": "/x/mine.sh"}}),
    );
    let env = configure(
        &ctx,
        "claude-code",
        &["--runtime", r#"{"claude_effort_level": "high"}"#],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        settings_file(&ctx)["statusLine"]["command"],
        "/x/mine.sh",
        "用户自己的状态栏原样留着"
    );
    assert!(
        !ctx.data_dir.join("statusline.sh").exists(),
        "没选内置就不部署脚本"
    );

    // 他换成别的脚本不改变我们的基准
    let before = base_of(&ctx, "claude-code");
    let mut file = settings_file(&ctx);
    file["statusLine"]["command"] = json!("/x/another.sh");
    write_settings_file(&ctx, &file);
    assert_eq!(base_of(&ctx, "claude-code"), before);

    let _ = fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn 配置文件是符号链接时写到最终目标_链接本身不变() {
    use std::os::unix::fs::symlink;
    let (base, ctx) = temp_ctx("symlink");
    let dotfiles = base.join("dotfiles");
    fs::create_dir_all(&dotfiles).unwrap();
    let real = dotfiles.join("claude-settings.json");
    fs::write(&real, br#"{"env": {"MY_OWN": "keep-me"}}"#).unwrap();
    let link = ctx.home_dir.join(".claude/settings.json");
    symlink(&real, &link).unwrap();

    assert_eq!(current_config(&ctx, "claude-code")["state"], "ready");
    let applied = configure(
        &ctx,
        "claude-code",
        &["--runtime", r#"{"claude_effort_level": "high"}"#],
    );
    assert!(applied.ok, "{:?}", applied.errors);
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), real);
    let written: Value = serde_json::from_slice(&fs::read(&real).unwrap()).unwrap();
    assert_eq!(written["effortLevel"], "high");
    assert_eq!(written["env"]["MY_OWN"], "keep-me");
    assert_eq!(
        dotfiles.read_dir().unwrap().count(),
        1,
        "不在目标目录留临时文件"
    );

    // 断链：拒绝，不凭空造文件
    fs::remove_file(&real).unwrap();
    let refused = configure_with_base(
        &ctx,
        "claude-code",
        &"0".repeat(64),
        &["--runtime", r#"{"claude_effort_level": "low"}"#],
    );
    assert!(!refused.ok);
    assert!(!real.exists());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 无法解析的配置文件拒绝写入_不当成空文件覆盖() {
    let (base, ctx) = temp_ctx("invalid-json");
    let broken = b"{\"env\": {\"MY_OWN\": \"keep-me\"},";
    fs::write(ctx.home_dir.join(".claude/settings.json"), broken).unwrap();
    let current = current_config(&ctx, "claude-code");
    assert_eq!(current["state"], "invalid");
    assert_eq!(current["base"], Value::Null, "没有基准，界面不能写");

    let refused = configure_with_base(
        &ctx,
        "claude-code",
        &"0".repeat(64),
        &["--runtime", r#"{"claude_effort_level": "high"}"#],
    );
    assert!(!refused.ok);
    assert!(refused.errors[0].message.contains("不是合法的 JSON"));
    assert_eq!(
        file_bytes(&ctx.home_dir.join(".claude/settings.json")),
        broken
    );

    let _ = fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn 写助手配置失败时回滚密钥文件与库() {
    use std::os::unix::fs::PermissionsExt;
    let (base, ctx) = temp_ctx("rollback");
    let secrets = configure_secret_source(&ctx);
    seed_live_token(&ctx);
    let secrets_before = file_bytes(&secrets);
    let settings_before = file_bytes(&ctx.home_dir.join(".claude/settings.json"));
    let policy_before = settings_in_db(&ctx, "claude-code");
    let base_digest = base_of(&ctx, "claude-code");

    // 助手配置目录只读：密钥文件已写入之后，settings.json 的原子替换失败
    let claude_dir = ctx.home_dir.join(".claude");
    fs::set_permissions(&claude_dir, fs::Permissions::from_mode(0o500)).unwrap();
    let failed = configure_with_base(&ctx, "claude-code", &base_digest, &["--use-default"]);
    fs::set_permissions(&claude_dir, fs::Permissions::from_mode(0o700)).unwrap();

    assert!(!failed.ok, "只读目录下不该写成功");
    assert_eq!(file_bytes(&secrets), secrets_before, "密钥文件回滚到原样");
    assert_eq!(
        file_bytes(&ctx.home_dir.join(".claude/settings.json")),
        settings_before
    );
    assert_eq!(settings_in_db(&ctx, "claude-code"), policy_before, "库不变");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 默认模型只改助手文件且外部修改会拒绝() {
    let (base, ctx) = temp_ctx("default-model");
    write_settings_file(
        &ctx,
        &json!({
            "model": "claude-sonnet-5",
            "env": {"ANTHROPIC_MODEL": "env-model"},
            "hooks": {"Stop": []}
        }),
    );
    let initial = current_config(&ctx, "claude-code");
    assert_eq!(
        initial["config"]["runtime"]["default_model"],
        "claude-sonnet-5"
    );
    assert_eq!(initial["config"]["model"], "env-model");
    let stale = initial["base"].as_str().unwrap().to_string();

    let mut external = settings_file(&ctx);
    external["model"] = json!("claude-opus-5");
    write_settings_file(&ctx, &external);
    let file_before = file_bytes(&ctx.home_dir.join(".claude/settings.json"));
    let db_before = db_bytes(&ctx);
    let refused = configure_with_base(
        &ctx,
        "claude-code",
        &stale,
        &["--runtime", r#"{"default_model":"claude-haiku-5"}"#],
    );
    assert!(!refused.ok);
    assert_eq!(refused.errors[0].code, "AGENT_CONFIG_CHANGED");
    assert_eq!(
        file_bytes(&ctx.home_dir.join(".claude/settings.json")),
        file_before
    );
    assert_eq!(db_bytes(&ctx), db_before);

    let applied = configure(
        &ctx,
        "claude-code",
        &["--runtime", r#"{"default_model":"claude-haiku-5"}"#],
    );
    assert!(applied.ok, "{:?}", applied.errors);
    let after = settings_file(&ctx);
    assert_eq!(after["model"], "claude-haiku-5");
    assert_eq!(after["env"]["ANTHROPIC_MODEL"], "env-model");
    assert_eq!(after["hooks"], json!({"Stop": []}));
    assert_eq!(
        current_config(&ctx, "claude-code")["config"]["runtime"]["default_model"],
        "claude-haiku-5"
    );
    assert!(settings_in_db(&ctx, "claude-code")
        .get("default_model")
        .is_none());

    let cleared = configure(
        &ctx,
        "claude-code",
        &["--runtime", r#"{"default_model":null}"#],
    );
    assert!(cleared.ok, "{:?}", cleared.errors);
    assert!(settings_file(&ctx).get("model").is_none());
    assert_eq!(settings_file(&ctx)["env"]["ANTHROPIC_MODEL"], "env-model");
    let _ = fs::remove_dir_all(&base);
}

// ---------- Codex ----------

#[test]
fn codex_默认模型不改登录与其他配置() {
    let (base, ctx) = temp_ctx("codex-default-model");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    fs::write(
        ctx.home_dir.join(".codex/config.toml"),
        "model = \"gpt-5.6-sol\"\nuser_instructions = \"keep\"\n",
    )
    .unwrap();
    let auth = ctx.home_dir.join(".codex/auth.json");
    fs::write(&auth, br#"{"test_only":"unchanged"}"#).unwrap();
    let auth_before = file_bytes(&auth);
    assert_eq!(
        current_config(&ctx, "codex")["config"]["runtime"]["default_model"],
        "gpt-5.6-sol"
    );

    let applied = configure(
        &ctx,
        "codex",
        &["--runtime", r#"{"default_model":"gpt-6-sol"}"#],
    );
    assert!(applied.ok, "{:?}", applied.errors);
    assert!(codex_config(&ctx).contains("model = \"gpt-6-sol\""));
    assert!(codex_config(&ctx).contains("user_instructions = \"keep\""));
    assert_eq!(file_bytes(&auth), auth_before);
    assert_eq!(
        current_config(&ctx, "codex")["config"]["runtime"]["default_model"],
        "gpt-6-sol"
    );
    assert!(settings_in_db(&ctx, "codex").get("default_model").is_none());

    let cleared = configure(&ctx, "codex", &["--runtime", r#"{"default_model":null}"#]);
    assert!(cleared.ok, "{:?}", cleared.errors);
    assert!(!codex_config(&ctx).contains("model = "));
    assert!(codex_config(&ctx).contains("user_instructions = \"keep\""));
    assert_eq!(file_bytes(&auth), auth_before);
    let _ = fs::remove_dir_all(&base);
}

/// 写一条 Codex 助手定义（配置文件与 MCP 同一个 config.toml）。
fn define_codex(ctx: &Context, settings: Value) {
    let definition = json!({
        "_replace": true,
        "display_name": "Codex",
        "bin": "codex",
        "mcp_config": {"path": "~/.codex/config.toml", "format": "toml"},
        "preferred_provider": "native",
        "providers": {"native": {"install": "true", "upgrade": "true", "managed": true}},
        "settings": settings,
    });
    let env = execute(
        ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.codex",
                "--value",
                &definition.to_string(),
            ],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
}

fn codex_config(ctx: &Context) -> String {
    fs::read_to_string(ctx.home_dir.join(".codex/config.toml")).unwrap()
}

/// 旧版本曾把第三方接入写进 Codex 主配置：写出当时的 config.toml，库里记下本工具写过的表。
/// 归属表由一次真实写入建出，结构与引擎一致。
fn seed_legacy_codex(ctx: &Context, settings: Value, config: &str, owned: &[&str]) {
    define_codex(ctx, settings);
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    let created = configure(
        ctx,
        "codex",
        &["--runtime", r#"{"codex_web_search": "live"}"#],
    );
    assert!(created.ok, "{:?}", created.errors);
    fs::write(ctx.home_dir.join(".codex/config.toml"), config).unwrap();
    let conn = rusqlite::Connection::open(store::db_path(ctx)).unwrap();
    conn.execute(
        "UPDATE agent_settings_bindings SET owned_providers = ?1 WHERE agent_id = 'codex'",
        [serde_json::to_string(owned).unwrap()],
    )
    .unwrap();
}

/// 旧版本为一套带凭据变量的接入写出的 provider 表。
fn legacy_codex_table(name: &str, base_url: &str, token_var: Option<&str>) -> String {
    let mut table =
        format!("\n[model_providers.{name}]\nname = \"{name}\"\nbase_url = \"{base_url}\"\n");
    if let Some(var) = token_var {
        table.push_str(&format!(
            "env_key = \"{var}\"\nrequires_openai_auth = true\n"
        ));
    }
    table.push_str("wire_api = \"responses\"\n");
    table
}

#[test]
fn codex_普通运行设置保留认证并拒绝把配置路径指向认证文件() {
    let (base, ctx) = temp_ctx("codex-auth-boundary");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    let auth_path = ctx.home_dir.join(".codex/auth.json");
    let auth_bytes = br#"{"auth_mode":"chatgpt","tokens":{"refresh_token":"test-only"}}"#;
    fs::write(&auth_path, auth_bytes).unwrap();

    let before = execute(&ctx, &request("agents:status", &[]));
    assert!(before.ok, "{:?}", before.errors);
    let codex = before.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == "agents:codex")
        .unwrap();
    // 状态不再读认证文件：没有 auth 字段，也不含其中任何内容（登录状态改由官方命令按需查询）
    assert!(codex["observed"].get("auth").is_none());
    assert!(!codex.to_string().contains("test-only"));

    define_codex(
        &ctx,
        json!({"active": "relay", "profiles": {
            "relay": {"base_url": "https://relay.example.com/v1", "token_var": "RELAY_KEY"},
            "local": {"base_url": "http://127.0.0.1:11434/v1"}
        }}),
    );
    let applied = configure(
        &ctx,
        "codex",
        &["--runtime", r#"{"codex_web_search": "live"}"#],
    );
    assert!(applied.ok, "{:?}", applied.errors);
    assert!(codex_config(&ctx).contains("web_search = \"live\""));
    assert_eq!(fs::read(&auth_path).unwrap(), auth_bytes);

    // 切回官方账号已退役：拒绝，认证文件照旧不动
    let cleared = configure(&ctx, "codex", &["--use-default"]);
    assert_eq!(cleared.errors[0].code, "AGENT_MAIN_OFFICIAL_ONLY");
    assert_eq!(fs::read(&auth_path).unwrap(), auth_bytes);

    let dangerous = json!({
        "_replace": true,
        "display_name": "Codex",
        "bin": "codex",
        "mcp_config": {"path": "~/.codex/auth.json", "format": "toml"},
        "preferred_provider": "native",
        "providers": {"native": {"install": "true", "upgrade": "true", "managed": true}},
        "settings": {"active": "relay", "profiles": {"relay": {"base_url": "https://other.example.com/v1"}}},
    });
    let saved = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.definitions.codex",
                "--value",
                &dangerous.to_string(),
            ],
        ),
    );
    assert!(saved.ok, "{:?}", saved.errors);

    let refused = configure(
        &ctx,
        "codex",
        &["--runtime", r#"{"codex_web_search": "cached"}"#],
    );
    assert!(!refused.ok);
    assert!(
        refused.errors[0].message.contains("只允许写 config.toml"),
        "{:?}",
        refused.errors
    );
    assert_eq!(fs::read(&auth_path).unwrap(), auth_bytes);

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn codex_写入只动自己那几项_用户的四百行配置原样保留() {
    let (base, ctx) = temp_ctx("codex-merge");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    // 用户已有的配置：顶层键、别的表段、注释
    fs::write(
        ctx.home_dir.join(".codex/config.toml"),
        concat!(
            "service_tier = \"default\"\n",
            "project_doc_max_bytes = 32768\n",
            "\n",
            "# 我自己加的插件\n",
            "[plugins.\"chrome@openai-bundled\"]\n",
            "enabled = true\n",
            "\n",
            "# [agent-manager:managed]\n",
            "[mcp_servers.grok]\n",
            "command = \"grok\"\n",
        ),
    )
    .unwrap();
    let env = configure(
        &ctx,
        "codex",
        &["--runtime", r#"{"codex_web_search": "live"}"#],
    );
    assert!(env.ok, "{:?}", env.errors);

    let after = codex_config(&ctx);
    // 用户的东西一个都没动
    assert!(after.contains("service_tier = \"default\""));
    assert!(after.contains("project_doc_max_bytes = 32768"));
    assert!(after.contains("# 我自己加的插件"));
    assert!(after.contains("[plugins.\"chrome@openai-bundled\"]"));
    // MCP 域写的那段也不能被我们碰
    assert!(
        after.contains("[mcp_servers.grok]"),
        "MCP 域的受管段不归我们管"
    );
    // 我们的东西写上了
    assert!(after.contains("web_search = \"live\""), "{after}");

    let _ = fs::remove_dir_all(&base);
}

fn owned_providers(ctx: &Context) -> Value {
    let conn = rusqlite::Connection::open(store::db_path(ctx)).unwrap();
    let raw: String = conn
        .query_row(
            "SELECT owned_providers FROM agent_settings_bindings WHERE agent_id = 'codex'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn codex_用户自己的_provider_在运行设置与切回官方时都逐字节保留() {
    let (base, ctx) = temp_ctx("codex-user-provider");
    let mine = concat!(
        "# 我自己的接入，别动\n",
        "[model_providers.mine]\n",
        "name = \"Mine\"\n",
        "base_url  =  \"https://mine.example.com/v1\"   # 手写的对齐\n",
        "\n",
        "[model_providers.mine.http_headers]\n",
        "X-Team = \"t\"\n",
    );
    let legacy = format!(
        "model_provider = \"a\"\n{mine}{}",
        legacy_codex_table("a", "https://a.example.com/v1", None)
    );
    seed_legacy_codex(
        &ctx,
        json!({"active": "a", "profiles": {"a": {"base_url": "https://a.example.com/v1"}}}),
        &legacy,
        &["a"],
    );

    let runtime = configure(
        &ctx,
        "codex",
        &["--runtime", r#"{"codex_web_search": "cached"}"#],
    );
    assert!(runtime.ok, "{:?}", runtime.errors);
    assert!(codex_config(&ctx).contains(mine), "{}", codex_config(&ctx));
    assert_eq!(owned_providers(&ctx), json!(["a"]), "运行设置不改归属记录");

    // 切回官方账号已退役：拒绝且文件与归属记录都不动
    let before_clear = codex_config(&ctx);
    let cleared = configure(&ctx, "codex", &["--use-default"]);
    assert_eq!(cleared.errors[0].code, "AGENT_MAIN_OFFICIAL_ONLY");
    assert_eq!(codex_config(&ctx), before_clear);
    assert_eq!(owned_providers(&ctx), json!(["a"]));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn codex_完全放开写审批与沙箱_切回手动写明安全值() {
    let (base, ctx) = temp_ctx("codex-yolo");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();

    let refused = configure(
        &ctx,
        "codex",
        &["--runtime", r#"{"automation_mode": "full"}"#],
    );
    assert!(!refused.ok, "降安全等级必须显式确认");

    let ok = configure(
        &ctx,
        "codex",
        &[
            "--runtime",
            r#"{"automation_mode": "full"}"#,
            "--allow-yolo",
        ],
    );
    assert!(ok.ok, "{:?}", ok.errors);
    let after = codex_config(&ctx);
    assert!(after.contains("approval_policy = \"never\""));
    assert!(after.contains("sandbox_mode = \"danger-full-access\""));

    // 切回手动：写出最终生效值，便于用户直接审查，而不是依赖客户端隐式默认。
    assert!(
        configure(
            &ctx,
            "codex",
            &["--runtime", r#"{"automation_mode": "manual"}"#]
        )
        .ok
    );
    let after = codex_config(&ctx);
    assert!(after.contains("approval_policy = \"on-request\""));
    assert!(after.contains("sandbox_mode = \"workspace-write\""));
    assert!(after.contains("approvals_reviewer = \"user\""));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn codex_智能自动保留沙箱并启用自动审查() {
    let (base, ctx) = temp_ctx("codex-auto");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();

    let result = configure(
        &ctx,
        "codex",
        &["--runtime", r#"{"automation_mode": "auto"}"#],
    );
    assert!(result.ok, "{:?}", result.errors);
    let after = codex_config(&ctx);
    assert!(after.contains("approval_policy = \"on-request\""));
    assert!(after.contains("sandbox_mode = \"workspace-write\""));
    assert!(after.contains("approvals_reviewer = \"auto_review\""));

    let _ = fs::remove_dir_all(&base);
}

// ---------- 可用性检查 ----------
