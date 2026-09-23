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

fn profiles_in_db(ctx: &Context, agent: &str) -> Value {
    let policy = execute(ctx, &request("policy:get", &[]));
    assert!(policy.ok, "{:?}", policy.errors);
    policy.data["effective"]["agents"]["definitions"][agent]["settings"]["profiles"].clone()
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

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
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
    define(
        &ctx,
        json!({"base_url": "https://api.example.com", "model": "claude-opus-5"}),
    );

    let env = configure(&ctx, "claude-code", &["--use", "p"]);
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.lock.held, "写动作要取排他锁");

    let after = settings_file(&ctx);
    assert!(
        after["hooks"]["SessionStart"].is_array(),
        "hooks 域的内容不能动"
    );
    assert_eq!(after["statusLine"]["command"], "/x/statusline.sh");
    assert_eq!(after["env"]["MY_OWN"], "keep-me");
    assert_eq!(
        after["env"]["ANTHROPIC_BASE_URL"],
        "https://api.example.com"
    );
    assert_eq!(after["env"]["ANTHROPIC_MODEL"], "claude-opus-5");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 缺_base_或写法不是恰好一种时拒绝_且零写() {
    let (base, ctx) = temp_ctx("usage");
    define(&ctx, json!({"base_url": "https://api.example.com"}));
    let db_before = db_bytes(&ctx);

    for args in [
        vec!["--only", "agents:claude-code", "--use", "p"],
        vec!["--only", "agents:claude-code", "--base", "x"],
        vec![
            "--only",
            "agents:claude-code",
            "--base",
            "x",
            "--use",
            "p",
            "--use-default",
        ],
        vec![
            "--only",
            "agents:claude-code",
            "--base",
            "x",
            "--save-current",
            "--connection",
            "{}",
        ],
    ] {
        let env = execute(&ctx, &request("agents:configure", &args));
        assert!(!env.ok, "{args:?}");
        assert_eq!(env.errors[0].code, "USAGE_ERROR", "{args:?}");
    }
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

#[test]
fn 外部改了地址_旧基准被拒_切换时存成导入项_切回写回外部地址() {
    let (base, ctx) = temp_ctx("external-url");
    define_settings(
        &ctx,
        json!({"active": "p", "profiles": {
            "p": {"base_url": "https://api.example.com"},
            "b": {"base_url": "https://b.example.com"}
        }}),
    );
    let ok = configure(&ctx, "claude-code", &["--use", "p"]);
    assert!(ok.ok, "{:?}", ok.errors);
    let stale = base_of(&ctx, "claude-code");

    // 本工具以外把地址改掉
    let mut current = settings_file(&ctx);
    current["env"]["ANTHROPIC_BASE_URL"] = json!("https://hand-edited.example.com");
    write_settings_file(&ctx, &current);
    let edited = file_bytes(&ctx.home_dir.join(".claude/settings.json"));

    // status 显示外部改后的地址；对不上任何已保存项，身份也变了
    let status = current_config(&ctx, "claude-code");
    assert_eq!(
        status["config"]["base_url"],
        "https://hand-edited.example.com"
    );
    assert_eq!(status["current"]["profile"], Value::Null);
    assert_eq!(status["current"]["last_written"], "p");
    assert_eq!(status["current"]["drifted_from"], Value::Null);

    // 旧基准：拒绝，文件与库一个字节都不动
    let db_before = db_bytes(&ctx);
    let refused = configure_with_base(&ctx, "claude-code", &stale, &["--use", "b"]);
    assert!(!refused.ok);
    assert_eq!(refused.errors[0].code, "AGENT_CONFIG_CHANGED");
    assert!(refused.errors[0].message.contains("已在外部修改"));
    assert_eq!(
        file_bytes(&ctx.home_dir.join(".claude/settings.json")),
        edited
    );
    assert_eq!(db_bytes(&ctx), db_before);

    // 新基准：先把外部地址存成导入项，再写目标
    let switched = configure(&ctx, "claude-code", &["--use", "b"]);
    assert!(switched.ok, "{:?}", switched.errors);
    assert_eq!(switched.data["backfill"]["created"], true);
    let imported = switched.data["backfill"]["name"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(imported.starts_with("imported-"), "{imported}");
    assert_eq!(
        profiles_in_db(&ctx, "claude-code")[&imported]["base_url"],
        "https://hand-edited.example.com"
    );
    assert_eq!(
        profiles_in_db(&ctx, "claude-code")["p"]["base_url"],
        "https://api.example.com",
        "原来那套不被覆盖"
    );
    assert_eq!(
        settings_file(&ctx)["env"]["ANTHROPIC_BASE_URL"],
        "https://b.example.com"
    );
    assert_eq!(
        current_config(&ctx, "claude-code")["current"]["profile"],
        "b"
    );

    // 切回导入项：外部地址写回
    let back = configure(&ctx, "claude-code", &["--use", &imported]);
    assert!(back.ok, "{:?}", back.errors);
    assert_eq!(back.data["backfill"], Value::Null, "b 已保存，不再导入");
    assert_eq!(
        settings_file(&ctx)["env"]["ANTHROPIC_BASE_URL"],
        "https://hand-edited.example.com"
    );
    assert_eq!(
        current_config(&ctx, "claude-code")["current"]["profile"],
        imported.as_str()
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 身份一致只改了模型_回填进上次写入的那套_不新建导入项() {
    let (base, ctx) = temp_ctx("identity");
    define_settings(
        &ctx,
        json!({"active": "relay", "profiles": {
            "relay": {"base_url": "https://relay.example.com", "model": "m1", "note": "工作线路"},
            "other": {"base_url": "https://other.example.com"}
        }}),
    );
    assert!(configure(&ctx, "claude-code", &["--use", "relay"]).ok);

    let mut current = settings_file(&ctx);
    current["env"]["ANTHROPIC_MODEL"] = json!("m2");
    current["env"]["API_TIMEOUT_MS"] = json!("600000");
    write_settings_file(&ctx, &current);
    let status = current_config(&ctx, "claude-code");
    assert_eq!(status["current"]["profile"], Value::Null);
    assert_eq!(status["current"]["drifted_from"], "relay");

    let switched = configure(&ctx, "claude-code", &["--use", "other"]);
    assert!(switched.ok, "{:?}", switched.errors);
    assert_eq!(switched.data["backfill"]["created"], false);
    assert_eq!(switched.data["backfill"]["name"], "relay");
    let profiles = profiles_in_db(&ctx, "claude-code");
    assert_eq!(profiles.as_object().unwrap().len(), 2, "不新建导入项");
    assert_eq!(profiles["relay"]["model"], "m2");
    assert_eq!(profiles["relay"]["timeout_ms"], 600000);
    assert_eq!(profiles["relay"]["note"], "工作线路", "未知字段原样保留");

    // 地址也换了：身份不同，新建导入项，relay 不被覆盖
    assert!(configure(&ctx, "claude-code", &["--use", "relay"]).ok);
    assert_eq!(settings_file(&ctx)["env"]["ANTHROPIC_MODEL"], "m2");
    let mut current = settings_file(&ctx);
    current["env"]["ANTHROPIC_BASE_URL"] = json!("https://third.example.com");
    write_settings_file(&ctx, &current);
    let switched = configure(&ctx, "claude-code", &["--use", "other"]);
    assert!(switched.ok, "{:?}", switched.errors);
    assert_eq!(switched.data["backfill"]["created"], true);
    assert_eq!(
        profiles_in_db(&ctx, "claude-code")["relay"]["base_url"],
        "https://relay.example.com"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 首次切换把用户手配的接入存成导入项_不丢() {
    let (base, ctx) = temp_ctx("first-run-existing");
    // 从没用本工具写过，但用户自己配了地址与模型
    write_settings_file(
        &ctx,
        &json!({"env": {"ANTHROPIC_BASE_URL": "https://mine.example.com", "ANTHROPIC_MODEL": "mine-model"}}),
    );
    define(&ctx, json!({"base_url": "https://api.example.com"}));

    let switched = configure(&ctx, "claude-code", &["--use", "p"]);
    assert!(switched.ok, "{:?}", switched.errors);
    let imported = switched.data["backfill"]["name"].as_str().unwrap();
    let saved = &profiles_in_db(&ctx, "claude-code")[imported];
    assert_eq!(saved["base_url"], "https://mine.example.com");
    assert_eq!(saved["model"], "mine-model");
    assert_eq!(
        settings_file(&ctx)["env"]["ANTHROPIC_BASE_URL"],
        "https://api.example.com"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 空库首次_保存当前实际配置只写库_不动助手文件() {
    let (base, ctx) = temp_ctx("save-current");
    let bytes = serde_json::to_vec_pretty(&json!({"env": {
        "ANTHROPIC_BASE_URL": "http://192.168.1.10:8080",
        "ANTHROPIC_MODEL": "lan-model"
    }}))
    .unwrap();
    fs::write(ctx.home_dir.join(".claude/settings.json"), &bytes).unwrap();
    assert_eq!(
        current_config(&ctx, "claude-code")["current"]["profile"],
        Value::Null
    );
    assert!(!store::db_path(&ctx).exists());

    let saved = configure(&ctx, "claude-code", &["--save-current"]);
    assert!(saved.ok, "{:?}", saved.errors);
    let imported = saved.data["backfill"]["name"].as_str().unwrap().to_string();
    // 局域网 http 中转：实际配置里已有的地址照样存得下
    assert_eq!(
        profiles_in_db(&ctx, "claude-code")[&imported]["base_url"],
        "http://192.168.1.10:8080"
    );
    assert_eq!(
        file_bytes(&ctx.home_dir.join(".claude/settings.json")),
        bytes
    );
    assert_eq!(
        current_config(&ctx, "claude-code")["current"]["profile"],
        imported.as_str()
    );
    // 已是保存过的当前项：再保存是空操作
    let again = configure(&ctx, "claude-code", &["--save-current"]);
    assert!(again.ok, "{:?}", again.errors);
    assert_eq!(again.data["backfill"], Value::Null);
    assert_eq!(
        profiles_in_db(&ctx, "claude-code")
            .as_object()
            .unwrap()
            .len(),
        1
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

fn assert_no_plaintext(ctx: &Context, envelopes: &[&Envelope]) {
    for (path, bytes) in data_dir_bytes(ctx) {
        assert!(
            !contains_bytes(&bytes, LIVE_TOKEN.as_bytes()),
            "数据目录里的 {} 出现了令牌明文",
            path.display()
        );
    }
    for request_name in ["policy:get", "runs:list"] {
        let env = execute(ctx, &request(request_name, &[]));
        assert!(!serde_json::to_string(&env.data)
            .unwrap()
            .contains(LIVE_TOKEN));
    }
    for envelope in envelopes {
        assert!(!serde_json::to_string(&envelope.data)
            .unwrap()
            .contains(LIVE_TOKEN));
        assert!(!format!("{:?}", envelope.errors).contains(LIVE_TOKEN));
    }
}

#[test]
fn 实际令牌没保存且没选密钥文件_切换被拒_文件与库不变() {
    let (base, ctx) = temp_ctx("token-refused");
    seed_live_token(&ctx);
    let before = file_bytes(&ctx.home_dir.join(".claude/settings.json"));
    let db_before = db_bytes(&ctx);

    let refused = configure(&ctx, "claude-code", &["--use", "p"]);
    assert!(!refused.ok);
    assert_eq!(refused.errors[0].code, "AGENT_LIVE_TOKEN_UNSAVED");
    assert!(refused.errors[0].message.contains("密钥"));
    assert_eq!(
        file_bytes(&ctx.home_dir.join(".claude/settings.json")),
        before
    );
    assert_eq!(db_bytes(&ctx), db_before);
    assert_no_plaintext(&ctx, &[&refused]);

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 选了密钥文件后切换_实际令牌写进密钥文件_库与记录没有明文_切回原样写回() {
    let (base, ctx) = temp_ctx("token-adopted");
    let secrets = configure_secret_source(&ctx);
    seed_live_token(&ctx);

    let switched = configure(&ctx, "claude-code", &["--use", "p"]);
    assert!(switched.ok, "{:?}", switched.errors);
    assert_eq!(switched.data["backfill"]["token"], "adopted");
    let var = switched.data["backfill"]["token_var"]
        .as_str()
        .unwrap()
        .to_string();
    let imported = switched.data["backfill"]["name"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(fs::read_to_string(&secrets)
        .unwrap()
        .contains(&format!("{var}=\"{LIVE_TOKEN}\"")));
    assert_eq!(
        profiles_in_db(&ctx, "claude-code")[&imported]["token_var"],
        var.as_str()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&secrets).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(settings_file(&ctx)["env"]
        .get("ANTHROPIC_AUTH_TOKEN")
        .is_none());

    let back = configure(&ctx, "claude-code", &["--use", &imported]);
    assert!(back.ok, "{:?}", back.errors);
    assert_eq!(
        settings_file(&ctx)["env"]["ANTHROPIC_AUTH_TOKEN"],
        LIVE_TOKEN
    );
    assert_eq!(
        current_config(&ctx, "claude-code")["config"]["credential"]["saved_var"],
        var.as_str()
    );
    assert_no_plaintext(&ctx, &[&switched, &back]);

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 显式放弃实际令牌_切换成功_库里没有明文() {
    let (base, ctx) = temp_ctx("token-discarded");
    seed_live_token(&ctx);

    let switched = configure(&ctx, "claude-code", &["--use", "p", "--discard-live-token"]);
    assert!(switched.ok, "{:?}", switched.errors);
    assert_eq!(switched.data["backfill"]["token"], "discarded");
    let imported = switched.data["backfill"]["name"].as_str().unwrap();
    let saved = &profiles_in_db(&ctx, "claude-code")[imported];
    assert_eq!(saved["base_url"], "https://hand.example.com");
    assert!(saved.get("token_var").is_none());
    assert_no_plaintext(&ctx, &[&switched]);

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 实际接入存不下时拒绝并说明_显式放弃后才切换() {
    let (base, ctx) = temp_ctx("unrepresentable");
    write_settings_file(
        &ctx,
        &json!({"env": {
            "ANTHROPIC_BASE_URL": "https://hand.example.com",
            "ANTHROPIC_AUTH_TOKEN": "a",
            "ANTHROPIC_API_KEY": "b"
        }}),
    );
    define(&ctx, json!({"base_url": "https://api.example.com"}));
    let before = file_bytes(&ctx.home_dir.join(".claude/settings.json"));
    let db_before = db_bytes(&ctx);

    let refused = configure(&ctx, "claude-code", &["--use", "p"]);
    assert!(!refused.ok);
    assert_eq!(refused.errors[0].code, "AGENT_LIVE_UNREPRESENTABLE");
    assert!(
        refused.errors[0]
            .message
            .contains("ANTHROPIC_AUTH_TOKEN 与 ANTHROPIC_API_KEY"),
        "{:?}",
        refused.errors
    );
    assert_eq!(
        file_bytes(&ctx.home_dir.join(".claude/settings.json")),
        before
    );
    assert_eq!(db_bytes(&ctx), db_before);

    let switched = configure(&ctx, "claude-code", &["--use", "p", "--discard-live"]);
    assert!(switched.ok, "{:?}", switched.errors);
    assert_eq!(switched.data["discarded_live"], true);
    assert_eq!(switched.data["backfill"], Value::Null);
    let after = settings_file(&ctx);
    assert_eq!(
        after["env"]["ANTHROPIC_BASE_URL"],
        "https://api.example.com"
    );
    assert!(after["env"].get("ANTHROPIC_API_KEY").is_none());
    assert_eq!(
        profiles_in_db(&ctx, "claude-code")
            .as_object()
            .unwrap()
            .len(),
        1
    );

    let _ = fs::remove_dir_all(&base);
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
    assert_eq!(after["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "low");
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
fn 令牌只存变量名_取不到值时拒绝写入_审计不记取值() {
    let (base, ctx) = temp_ctx("token");
    configure_secret_source(&ctx);
    define(&ctx, json!({"token_var": "MY_GATEWAY_TOKEN"}));

    let refused = configure(&ctx, "claude-code", &["--use", "p"]);
    assert!(!refused.ok, "变量没取值就写上去，助手会以为配好了");
    assert!(refused.errors[0].message.contains("MY_GATEWAY_TOKEN"));

    // 在密钥页填上取值
    const SECRET: &str = "sk-gateway-secret-value";
    let set = execute(
        &ctx,
        &request(
            "secrets:set",
            &["--var", "MY_GATEWAY_TOKEN", "--value", SECRET],
        ),
    );
    assert!(set.ok, "{:?}", set.errors);

    let ok = configure(&ctx, "claude-code", &["--use", "p"]);
    assert!(ok.ok, "{:?}", ok.errors);
    // settings.json 的 env 不做变量展开，只能落真值；库里存的仍是变量名
    assert_eq!(settings_file(&ctx)["env"]["ANTHROPIC_AUTH_TOKEN"], SECRET);
    let policy = execute(&ctx, &request("policy:get", &[]));
    let stored = serde_json::to_string(&policy.data).unwrap();
    assert!(!stored.contains(SECRET), "定义库里不许出现取值");
    assert!(stored.contains("MY_GATEWAY_TOKEN"), "库里存的是变量名");
    // 审计只记变量名，不记取值
    let runs = execute(&ctx, &request("runs:list", &[]));
    assert!(!serde_json::to_string(&runs.data).unwrap().contains(SECRET));
    assert!(!serde_json::to_string(&ok.data).unwrap().contains(SECRET));
    // 当前接入按内容识别：令牌变量的解析值与实际一致
    let current = current_config(&ctx, "claude-code");
    assert_eq!(current["current"]["profile"], "p");
    assert_eq!(
        current["config"]["credential"]["saved_var"],
        "MY_GATEWAY_TOKEN"
    );

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

    let refused = configure(&ctx, "opencode", &["--use", "p"]);
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
        let refused = configure(&ctx, "claude-code", &["--use", "p"]);
        assert!(!refused.ok, "非法输入不该落地");
        assert!(!ctx.home_dir.join(".claude/settings.json").exists());
    }
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 切回默认只撤接入键_不碰别人的() {
    let (base, ctx) = temp_ctx("clear");
    define(
        &ctx,
        json!({"base_url": "https://api.example.com", "model": "m1"}),
    );
    assert!(configure(&ctx, "claude-code", &["--use", "p"]).ok);

    // 用户随后往里加了自己的东西
    let mut current = settings_file(&ctx);
    current["statusLine"] = json!({"type": "command", "command": "/x/s.sh"});
    current["env"]["MY_OWN"] = json!("keep-me");
    write_settings_file(&ctx, &current);

    let cleared = configure(&ctx, "claude-code", &["--use-default"]);
    assert!(cleared.ok, "{:?}", cleared.errors);
    assert_eq!(
        cleared.data["backfill"],
        Value::Null,
        "实际仍是 p，不用回填"
    );

    let after = settings_file(&ctx);
    assert!(after["env"].get("ANTHROPIC_BASE_URL").is_none());
    assert!(after["env"].get("ANTHROPIC_MODEL").is_none());
    assert_eq!(after["env"]["MY_OWN"], "keep-me");
    assert_eq!(after["statusLine"]["command"], "/x/s.sh");

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
fn 切换接入配置一步到位_库里与文件一起变() {
    let (base, ctx) = temp_ctx("switch");
    define_settings(
        &ctx,
        json!({
            "active": "official",
            "profiles": {
                "official": {"label": "官方"},
                "relay": {"label": "我的中转", "base_url": "https://relay.example.com", "model": "relay-model"}
            }
        }),
    );
    // 先按 official 落一次
    assert!(configure(&ctx, "claude-code", &["--use", "official"]).ok);
    assert!(
        !ctx.home_dir.join(".claude/settings.json").exists()
            || settings_file(&ctx).get("env").is_none(),
        "官方配置不需要环境变量"
    );

    // 切到中转：一条命令既改库也写文件
    let switched = configure(&ctx, "claude-code", &["--use", "relay"]);
    assert!(switched.ok, "{:?}", switched.errors);
    assert_eq!(switched.data["active"], "relay");
    assert_eq!(switched.data["current"], "relay");
    let after = settings_file(&ctx);
    assert_eq!(
        after["env"]["ANTHROPIC_BASE_URL"],
        "https://relay.example.com"
    );
    assert_eq!(after["env"]["ANTHROPIC_MODEL"], "relay-model");

    // 库里的上次写入记录也跟着变了
    assert_eq!(settings_in_db(&ctx, "claude-code")["active"], "relay");

    // 再切回官方：上一套留下的变量一个都不许剩
    let back = configure(&ctx, "claude-code", &["--use", "official"]);
    assert!(back.ok, "{:?}", back.errors);
    assert!(
        settings_file(&ctx).get("env").is_none(),
        "切回后不得残留上一套的变量"
    );
    assert_eq!(
        current_config(&ctx, "claude-code")["current"]["profile"],
        "official"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 切到不存在的接入配置被拒绝_且不动文件() {
    let (base, ctx) = temp_ctx("switch-missing");
    define_settings(
        &ctx,
        json!({"active": "p", "profiles": {"p": {"base_url": "https://api.example.com"}}}),
    );
    assert!(configure(&ctx, "claude-code", &["--use", "p"]).ok);
    let before = fs::read(ctx.home_dir.join(".claude/settings.json")).unwrap();

    let refused = configure(&ctx, "claude-code", &["--use", "nope"]);
    assert!(!refused.ok);
    assert_eq!(
        fs::read(ctx.home_dir.join(".claude/settings.json")).unwrap(),
        before,
        "被拒时一个字节都不许动"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 编辑当前接入_写文件并更新库_旧基准被拒() {
    let (base, ctx) = temp_ctx("edit-current");
    define(
        &ctx,
        json!({"base_url": "https://old.example.com", "model": "m1", "note": "备注"}),
    );
    assert!(configure(&ctx, "claude-code", &["--use", "p"]).ok);
    let stale = base_of(&ctx, "claude-code");
    // 外部顺手加了小任务模型：旧基准作废
    let mut current = settings_file(&ctx);
    current["env"]["ANTHROPIC_SMALL_FAST_MODEL"] = json!("small");
    write_settings_file(&ctx, &current);
    let edited = file_bytes(&ctx.home_dir.join(".claude/settings.json"));
    let connection = r#"{"label": "新名字", "note": "备注", "base_url": "https://new.example.com", "model": "m2"}"#;

    let refused = configure_with_base(
        &ctx,
        "claude-code",
        &stale,
        &["--use", "p", "--connection", connection],
    );
    assert!(!refused.ok);
    assert_eq!(refused.errors[0].code, "AGENT_CONFIG_CHANGED");
    assert_eq!(
        file_bytes(&ctx.home_dir.join(".claude/settings.json")),
        edited
    );
    assert_eq!(
        profiles_in_db(&ctx, "claude-code")["p"]["base_url"],
        "https://old.example.com"
    );

    let applied = configure(
        &ctx,
        "claude-code",
        &["--use", "p", "--connection", connection],
    );
    assert!(applied.ok, "{:?}", applied.errors);
    let after = settings_file(&ctx);
    assert_eq!(
        after["env"]["ANTHROPIC_BASE_URL"],
        "https://new.example.com"
    );
    assert_eq!(after["env"]["ANTHROPIC_MODEL"], "m2");
    assert!(after["env"].get("ANTHROPIC_SMALL_FAST_MODEL").is_none());
    let saved = &profiles_in_db(&ctx, "claude-code")["p"];
    assert_eq!(saved["label"], "新名字");
    assert_eq!(saved["base_url"], "https://new.example.com");
    assert_eq!(saved["note"], "备注");
    assert_eq!(
        profiles_in_db(&ctx, "claude-code")
            .as_object()
            .unwrap()
            .len(),
        1,
        "编辑当前那套不产生导入项"
    );

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
    define_settings(
        &ctx,
        json!({"active": "p", "profiles": {"p": {"base_url": "https://api.example.com"}}}),
    );

    let env = configure(&ctx, "claude-code", &["--use", "p"]);
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

#[test]
fn 两种认证方式各写各的环境变量_编辑当前那套立即生效() {
    let (base, ctx) = temp_ctx("auth");
    configure_secret_source(&ctx);
    const SECRET: &str = "sk-relay-secret";
    assert!(
        execute(
            &ctx,
            &request("secrets:set", &["--var", "RELAY_KEY", "--value", SECRET])
        )
        .ok
    );

    define_settings(
        &ctx,
        json!({"active": "p", "profiles": {"p": {"token_var": "RELAY_KEY", "auth": "api_key"}}}),
    );
    assert!(configure(&ctx, "claude-code", &["--use", "p"]).ok);
    let after = settings_file(&ctx);
    assert_eq!(after["env"]["ANTHROPIC_API_KEY"], SECRET);
    assert!(after["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());

    // 换成 Bearer：上一个变量要消失，否则两个头一起发出去
    let edited = configure(
        &ctx,
        "claude-code",
        &[
            "--use",
            "p",
            "--connection",
            r#"{"token_var": "RELAY_KEY", "auth": "token"}"#,
        ],
    );
    assert!(edited.ok, "{:?}", edited.errors);
    let after = settings_file(&ctx);
    assert_eq!(after["env"]["ANTHROPIC_AUTH_TOKEN"], SECRET);
    assert!(after["env"].get("ANTHROPIC_API_KEY").is_none());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 切回默认会撤掉接入配置_库里的上次写入记录也清空() {
    let (base, ctx) = temp_ctx("use-default");
    define_settings(
        &ctx,
        json!({
            "active": "relay",
            "profiles": {"relay": {"base_url": "https://relay.example.com"}}
        }),
    );
    assert!(configure(&ctx, "claude-code", &["--use", "relay"]).ok);
    assert_eq!(
        settings_file(&ctx)["env"]["ANTHROPIC_BASE_URL"],
        "https://relay.example.com"
    );

    let back = configure(&ctx, "claude-code", &["--use-default"]);
    assert!(back.ok, "{:?}", back.errors);
    assert_eq!(back.data["active"], Value::Null);
    assert!(
        settings_file(&ctx).get("env").is_none(),
        "切回默认后不留任何接入变量"
    );
    assert!(settings_in_db(&ctx, "claude-code").get("active").is_none());
    assert_eq!(
        current_config(&ctx, "claude-code")["current"]["default"],
        true
    );

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
    define(&ctx, json!({"base_url": "https://api.example.com"}));

    assert_eq!(current_config(&ctx, "claude-code")["state"], "ready");
    let applied = configure(&ctx, "claude-code", &["--use", "p"]);
    assert!(applied.ok, "{:?}", applied.errors);
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), real);
    let written: Value = serde_json::from_slice(&fs::read(&real).unwrap()).unwrap();
    assert_eq!(
        written["env"]["ANTHROPIC_BASE_URL"],
        "https://api.example.com"
    );
    assert_eq!(written["env"]["MY_OWN"], "keep-me");
    assert_eq!(
        dotfiles.read_dir().unwrap().count(),
        1,
        "不在目标目录留临时文件"
    );

    // 断链：拒绝，不凭空造文件
    fs::remove_file(&real).unwrap();
    let refused = configure_with_base(&ctx, "claude-code", &"0".repeat(64), &["--use", "p"]);
    assert!(!refused.ok);
    assert!(!real.exists());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 无法解析的配置文件拒绝写入_不当成空文件覆盖() {
    let (base, ctx) = temp_ctx("invalid-json");
    let broken = b"{\"env\": {\"MY_OWN\": \"keep-me\"},";
    fs::write(ctx.home_dir.join(".claude/settings.json"), broken).unwrap();
    define(&ctx, json!({"base_url": "https://api.example.com"}));
    let current = current_config(&ctx, "claude-code");
    assert_eq!(current["state"], "invalid");
    assert_eq!(current["base"], Value::Null, "没有基准，界面不能写");

    let refused = configure_with_base(&ctx, "claude-code", &"0".repeat(64), &["--use", "p"]);
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
    let failed = configure_with_base(&ctx, "claude-code", &base_digest, &["--use", "p"]);
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

// ---------- Codex ----------

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
    assert_eq!(codex["observed"]["auth"]["state"], "ready");
    assert_eq!(codex["observed"]["auth"]["mode"], "chatgpt");
    assert!(!codex["observed"]["auth"].to_string().contains("test-only"));

    define_codex(
        &ctx,
        json!({"active": "relay", "profiles": {
            "relay": {"base_url": "https://relay.example.com/v1", "token_var": "RELAY_KEY"},
            "local": {"base_url": "http://127.0.0.1:11434/v1"}
        }}),
    );
    let applied = configure(&ctx, "codex", &["--use", "relay"]);
    assert!(applied.ok, "{:?}", applied.errors);
    let config = codex_config(&ctx);
    assert!(config.contains("env_key = \"RELAY_KEY\""));
    assert!(config.contains("requires_openai_auth = true"));
    assert_eq!(fs::read(&auth_path).unwrap(), auth_bytes);

    let keyless = configure(&ctx, "codex", &["--use", "local"]);
    assert!(keyless.ok, "{:?}", keyless.errors);
    let keyless_config = codex_config(&ctx);
    assert!(!keyless_config.contains("env_key ="));
    assert!(!keyless_config.contains("requires_openai_auth"));
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

    let refused = configure(&ctx, "codex", &["--use", "relay"]);
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
    define_codex(
        &ctx,
        json!({
            "active": "relay",
            "profiles": {"relay": {"label": "我的中转", "base_url": "https://relay.example.com/v1", "token_var": "RELAY_KEY", "model": "gpt-5"}}
        }),
    );

    let env = configure(&ctx, "codex", &["--use", "relay"]);
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
    assert!(after.contains("[model_providers.relay]"), "{after}");
    assert!(after.contains("base_url = \"https://relay.example.com/v1\""));
    assert!(
        after.contains("env_key = \"RELAY_KEY\""),
        "令牌只写变量名，Codex 自己去读"
    );
    assert!(after.contains("wire_api = \"responses\""));
    assert!(after.contains("model_provider = \"relay\""));
    assert!(after.contains("model = \"gpt-5\""));
    // 令牌取值不该出现在配置里（Codex 用 env_key 指向变量名）
    assert!(!after.contains("sk-"));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn codex_切换与切回默认_上一套的接入方式不残留() {
    let (base, ctx) = temp_ctx("codex-switch");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    define_codex(
        &ctx,
        json!({
            "active": "a",
            "profiles": {
                "a": {"base_url": "https://a.example.com/v1"},
                "b": {"base_url": "https://b.example.com/v1"}
            }
        }),
    );
    assert!(configure(&ctx, "codex", &["--use", "a"]).ok);
    assert!(codex_config(&ctx).contains("[model_providers.a]"));

    let switched = configure(&ctx, "codex", &["--use", "b"]);
    assert!(switched.ok, "{:?}", switched.errors);
    let after = codex_config(&ctx);
    assert!(after.contains("[model_providers.b]"));
    assert!(
        !after.contains("[model_providers.a]"),
        "上一套必须撤干净：{after}"
    );
    assert!(after.contains("model_provider = \"b\""));
    assert_eq!(
        after.matches("model_provider =").count(),
        1,
        "不能留下两行赋值"
    );

    let back = configure(&ctx, "codex", &["--use-default"]);
    assert!(back.ok, "{:?}", back.errors);
    let after = codex_config(&ctx);
    assert!(
        !after.contains("model_providers"),
        "切回默认后不留接入方式：{after}"
    );
    assert!(!after.contains("model_provider ="));

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

/// 表结构：(列名, 类型, 非空, 默认值, 主键序号)，按列顺序。
fn bindings_schema(ctx: &Context) -> Vec<(String, String, bool, Option<String>, i64)> {
    let conn = rusqlite::Connection::open(store::db_path(ctx)).unwrap();
    let mut stmt = conn
        .prepare("PRAGMA table_info(agent_settings_bindings)")
        .unwrap();
    stmt.query_map([], |row| {
        Ok((
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
        ))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

fn has_owned_providers_column(ctx: &Context) -> bool {
    bindings_schema(ctx)
        .iter()
        .any(|(name, ..)| name == "owned_providers")
}

#[test]
fn codex_用户自己的_provider_在写入_切换_清空后都逐字节保留() {
    let (base, ctx) = temp_ctx("codex-user-provider");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    let mine = concat!(
        "# 我自己的接入，别动\n",
        "[model_providers.mine]\n",
        "name = \"Mine\"\n",
        "base_url  =  \"https://mine.example.com/v1\"   # 手写的对齐\n",
        "\n",
        "[model_providers.mine.http_headers]\n",
        "X-Team = \"t\"\n",
    );
    let path = ctx.home_dir.join(".codex/config.toml");
    fs::write(&path, mine).unwrap();
    define_codex(
        &ctx,
        json!({
            "active": "a",
            "profiles": {
                "a": {"base_url": "https://a.example.com/v1"},
                "b": {"base_url": "https://b.example.com/v1"}
            }
        }),
    );

    let applied = configure(&ctx, "codex", &["--use", "a"]);
    assert!(applied.ok, "{:?}", applied.errors);
    assert!(codex_config(&ctx).contains(mine), "{}", codex_config(&ctx));
    assert_eq!(owned_providers(&ctx), json!(["a"]));

    let switched = configure(&ctx, "codex", &["--use", "b"]);
    assert!(switched.ok, "{:?}", switched.errors);
    assert!(codex_config(&ctx).contains(mine), "{}", codex_config(&ctx));
    assert_eq!(owned_providers(&ctx), json!(["b"]));

    let cleared = configure(&ctx, "codex", &["--use-default"]);
    assert!(cleared.ok, "{:?}", cleared.errors);
    let after = codex_config(&ctx);
    assert!(after.contains(mine), "{after}");
    assert!(!after.contains("[model_providers.b]"), "{after}");
    assert_eq!(owned_providers(&ctx), json!([]));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn codex_从_a_切到_b_按库里记录撤表_文件不留本工具注释() {
    let (base, ctx) = temp_ctx("codex-owned-switch");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    define_codex(
        &ctx,
        json!({
            "active": "a",
            "profiles": {
                "a": {"base_url": "https://a.example.com/v1", "token_var": "A_KEY"},
                "b": {"base_url": "https://b.example.com/v1"}
            }
        }),
    );
    assert!(configure(&ctx, "codex", &["--use", "a"]).ok);
    let first = codex_config(&ctx);
    assert!(first.contains("[model_providers.a]"));
    assert!(!first.contains("agent-manager"), "{first}");
    assert_eq!(owned_providers(&ctx), json!(["a"]));

    let switched = configure(&ctx, "codex", &["--use", "b"]);
    assert!(switched.ok, "{:?}", switched.errors);
    let after = codex_config(&ctx);
    assert!(!after.contains("[model_providers.a]"), "{after}");
    assert!(!after.contains("A_KEY"), "{after}");
    assert!(after.contains("[model_providers.b]"), "{after}");
    assert!(!after.contains("agent-manager"), "{after}");
    assert_eq!(owned_providers(&ctx), json!(["b"]));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn codex_旧库缺列加旧注释文件_用状态给的基准切换_写入后补列并迁走注释() {
    let (base, ctx) = temp_ctx("codex-legacy-db");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    define_codex(
        &ctx,
        json!({
            "active": "relay",
            "profiles": {
                "relay": {"base_url": "https://relay.example.com/v1", "token_var": "RELAY_KEY"},
                "next": {"base_url": "https://next.example.com/v1"}
            }
        }),
    );
    assert!(configure(&ctx, "codex", &["--use", "relay"]).ok);

    let fresh_schema = bindings_schema(&ctx);
    assert_eq!(
        fresh_schema.last().map(|col| col.0.as_str()),
        Some("owned_providers")
    );

    // 还原成旧版本留下的样子：provider 表前带受管注释，库里的表没有 owned_providers 列
    let path = ctx.home_dir.join(".codex/config.toml");
    let legacy = format!(
        "{}# [agent-manager:managed]\n[mcp_servers.grok]\ncommand = \"grok\"\n# 我的注释\n[model_providers.mine]\nbase_url = \"https://mine.example.com/v1\"\n",
        codex_config(&ctx).replace(
            "[model_providers.relay]",
            "# [agent-manager:managed]\n[model_providers.relay]"
        )
    );
    fs::write(&path, &legacy).unwrap();
    {
        let conn = rusqlite::Connection::open(store::db_path(&ctx)).unwrap();
        conn.execute_batch(
            "CREATE TABLE legacy_bindings (
                agent_id TEXT PRIMARY KEY NOT NULL,
                file TEXT NOT NULL,
                applied_digest TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            INSERT INTO legacy_bindings SELECT agent_id, file, applied_digest, updated_at
                FROM agent_settings_bindings;
            DROP TABLE agent_settings_bindings;
            ALTER TABLE legacy_bindings RENAME TO agent_settings_bindings;",
        )
        .unwrap();
    }
    assert!(!has_owned_providers_column(&ctx));

    // 查询零写：状态读取不补列；旧注释认领的表照样认得是 relay
    let current = current_config(&ctx, "codex");
    assert_eq!(current["current"]["profile"], "relay");
    assert!(!has_owned_providers_column(&ctx), "查询不得写库");
    assert_eq!(fs::read_to_string(&path).unwrap(), legacy);

    let switched = configure(&ctx, "codex", &["--use", "next"]);
    assert!(switched.ok, "{:?}", switched.errors);
    assert_eq!(switched.data["backfill"], Value::Null);
    assert_eq!(
        bindings_schema(&ctx),
        fresh_schema,
        "补列库与新建库结构一致"
    );
    assert_eq!(owned_providers(&ctx), json!(["next"]));

    let after = codex_config(&ctx);
    assert!(!after.contains("[model_providers.relay]"), "{after}");
    assert!(after.contains("[model_providers.next]"), "{after}");
    assert!(
        after.contains(
            "# 我的注释\n[model_providers.mine]\nbase_url = \"https://mine.example.com/v1\"\n"
        ),
        "{after}"
    );
    assert!(
        after.contains("# [agent-manager:managed]\n[mcp_servers.grok]\n"),
        "MCP 域的注释不归这里管：{after}"
    );
    assert_eq!(
        after.matches("# [agent-manager:managed]").count(),
        1,
        "{after}"
    );

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

#[test]
fn codex_内置接入方式名被拒_缺地址也被拒() {
    let (base, ctx) = temp_ctx("codex-reserved");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();

    // openai / ollama / lmstudio 是 Codex 内置的，覆盖不了
    define_codex(
        &ctx,
        json!({"active": "openai", "profiles": {"openai": {"base_url": "https://x.example.com/v1"}}}),
    );
    let refused = configure(&ctx, "codex", &["--use", "openai"]);
    assert!(!refused.ok, "内置名字不该放行");

    // 自定义接入方式没有默认地址，只填令牌变量名不成立
    define_codex(
        &ctx,
        json!({"active": "relay", "profiles": {"relay": {"token_var": "RELAY_KEY"}}}),
    );
    let refused = configure(&ctx, "codex", &["--use", "relay"]);
    assert!(!refused.ok, "缺地址要当场说清楚");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn codex_外部改了本工具写的表地址_旧基准被拒_切换时存成导入项_切回写回() {
    let (base, ctx) = temp_ctx("codex-external");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    define_codex(
        &ctx,
        json!({"active": "relay", "profiles": {
            "relay": {"base_url": "https://relay.example.com/v1", "token_var": "RELAY_KEY", "model": "gpt-5"},
            "b": {"base_url": "https://b.example.com/v1"}
        }}),
    );
    assert!(configure(&ctx, "codex", &["--use", "relay"]).ok);
    let stale = base_of(&ctx, "codex");

    // 本工具以外把地址改掉
    let path = ctx.home_dir.join(".codex/config.toml");
    let edited = codex_config(&ctx).replace(
        "https://relay.example.com/v1",
        "https://hand.example.com/v1",
    );
    fs::write(&path, &edited).unwrap();
    let status = current_config(&ctx, "codex");
    assert_eq!(status["config"]["base_url"], "https://hand.example.com/v1");
    assert_eq!(status["current"]["profile"], Value::Null);

    let refused = configure_with_base(&ctx, "codex", &stale, &["--use", "b"]);
    assert!(!refused.ok);
    assert_eq!(refused.errors[0].code, "AGENT_CONFIG_CHANGED");
    assert_eq!(codex_config(&ctx), edited, "拒绝时一个字节都不许动");

    let switched = configure(&ctx, "codex", &["--use", "b"]);
    assert!(switched.ok, "{:?}", switched.errors);
    assert_eq!(switched.data["backfill"]["created"], true);
    let imported = switched.data["backfill"]["name"]
        .as_str()
        .unwrap()
        .to_string();
    let saved = &profiles_in_db(&ctx, "codex")[&imported];
    assert_eq!(saved["base_url"], "https://hand.example.com/v1");
    assert_eq!(saved["token_var"], "RELAY_KEY");
    assert_eq!(saved["model"], "gpt-5");
    let after = codex_config(&ctx);
    assert!(!after.contains("[model_providers.relay]"), "{after}");
    assert!(after.contains("[model_providers.b]"), "{after}");
    assert_eq!(owned_providers(&ctx), json!(["b"]));

    let back = configure(&ctx, "codex", &["--use", &imported]);
    assert!(back.ok, "{:?}", back.errors);
    let after = codex_config(&ctx);
    assert!(
        after.contains("base_url = \"https://hand.example.com/v1\""),
        "{after}"
    );
    assert!(
        after.contains(&format!("model_provider = \"{imported}\"")),
        "{after}"
    );
    assert_eq!(owned_providers(&ctx), json!([imported]));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn codex_首次切换保留用户自己的运行参数_模型存进导入项() {
    let (base, ctx) = temp_ctx("codex-first-run");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    // 从没用本工具写过，但用户自己设了模型与审批策略
    fs::write(
        ctx.home_dir.join(".codex/config.toml"),
        "model = \"gpt-5.6-sol\"\napproval_policy = \"never\"\n",
    )
    .unwrap();
    define_codex(
        &ctx,
        json!({"active": "relay", "profiles": {"relay": {"base_url": "https://relay.example.com/v1", "model": "other"}}}),
    );
    let runtime = current_config(&ctx, "codex")["config"]["runtime"].clone();
    assert_eq!(runtime["automation_mode"], "custom");
    assert_eq!(runtime["automation_raw"]["approval_policy"], "never");

    let switched = configure(&ctx, "codex", &["--use", "relay"]);
    assert!(switched.ok, "{:?}", switched.errors);
    let imported = switched.data["backfill"]["name"].as_str().unwrap();
    assert_eq!(
        profiles_in_db(&ctx, "codex")[imported]["model"],
        "gpt-5.6-sol"
    );
    let after = codex_config(&ctx);
    assert!(
        after.contains("approval_policy = \"never\""),
        "切换不碰运行参数：{after}"
    );
    assert!(after.contains("model = \"other\""), "{after}");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn codex_用户自己的当前接入方式只被引用_不复制不认领_切回只写顶层键() {
    let (base, ctx) = temp_ctx("codex-reference");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    let user_table = concat!(
        "# 我自己的线路\n",
        "[model_providers.kimi]\n",
        "name = \"Kimi\"\n",
        "base_url = \"http://10.0.0.2:8080/v1\"\n",
        "env_key = \"KIMI_KEY\"\n",
        "\n",
        "[model_providers.kimi.http_headers]\n",
        "X-Team = \"t\"\n",
    );
    let path = ctx.home_dir.join(".codex/config.toml");
    fs::write(
        &path,
        format!("model_provider = \"kimi\"\nmodel = \"k2\"\n\n{user_table}"),
    )
    .unwrap();
    define_codex(
        &ctx,
        json!({"profiles": {"relay": {"base_url": "https://relay.example.com/v1"}}}),
    );
    assert_eq!(
        current_config(&ctx, "codex")["current"]["profile"],
        Value::Null
    );

    let switched = configure(&ctx, "codex", &["--use", "relay"]);
    assert!(switched.ok, "{:?}", switched.errors);
    let imported = switched.data["backfill"]["name"]
        .as_str()
        .unwrap()
        .to_string();
    let saved = &profiles_in_db(&ctx, "codex")[&imported];
    assert_eq!(saved["provider_ref"], "kimi");
    assert_eq!(saved["model"], "k2");
    assert!(saved.get("base_url").is_none(), "不复制那张表");
    assert!(
        codex_config(&ctx).contains(user_table),
        "用户的表逐字节保留"
    );
    assert_eq!(owned_providers(&ctx), json!(["relay"]));

    let back = configure(&ctx, "codex", &["--use", &imported]);
    assert!(back.ok, "{:?}", back.errors);
    let after = codex_config(&ctx);
    assert!(after.contains("model_provider = \"kimi\""), "{after}");
    assert!(after.contains("model = \"k2\""), "{after}");
    assert!(after.contains(user_table), "{after}");
    assert!(!after.contains("[model_providers.relay]"), "{after}");
    assert_eq!(owned_providers(&ctx), json!([]), "引用不认领");
    assert_eq!(
        current_config(&ctx, "codex")["current"]["profile"],
        imported.as_str()
    );

    // 引用的表被用户删掉之后：切到它拒绝并说明
    assert!(configure(&ctx, "codex", &["--use", "relay"]).ok);
    let without = codex_config(&ctx).replace(user_table, "");
    fs::write(&path, &without).unwrap();
    let refused = configure(&ctx, "codex", &["--use", &imported]);
    assert!(!refused.ok);
    assert!(
        refused.errors[0].message.contains("已不在 config.toml 中"),
        "{:?}",
        refused.errors
    );
    assert_eq!(codex_config(&ctx), without);

    let _ = fs::remove_dir_all(&base);
}

/// 库里 agent_settings_bindings 有没有 codex 这一行；表不存在也算没有。
fn has_codex_binding(ctx: &Context) -> bool {
    let conn = rusqlite::Connection::open(store::db_path(ctx)).unwrap();
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agent_settings_bindings'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    exists == 1
        && conn
            .query_row(
                "SELECT COUNT(*) FROM agent_settings_bindings WHERE agent_id = 'codex'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
}

fn header_count(text: &str, header: &str) -> usize {
    text.lines().filter(|line| line.trim() == header).count()
}

/// 用户手写的同名 relay（带子表），旁边还有别的 provider 与同名 MCP 表。
const USER_RELAY: &str = concat!(
    "# 我自己的 relay\n",
    "[model_providers.relay]\n",
    "name = \"Hand Relay\"\n",
    "base_url = \"https://hand.example.com/v1\"\n",
    "\n",
    "[model_providers.relay.http_headers]\n",
    "X-Hand = \"1\"\n",
    "\n",
);
const USER_OTHERS: &str = concat!(
    "[model_providers.mine]\n",
    "name = \"Mine\"\n",
    "base_url  =  \"https://mine.example.com/v1\"   # 手写的对齐\n",
    "\n",
    "[mcp_servers.relay]\n",
    "command = \"relay-mcp\"\n",
);

#[test]
fn codex_用户自己的同名_provider_写入被拒_换名后写入且用户表逐字节保留() {
    let (base, ctx) = temp_ctx("codex-provider-collision");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    let path = ctx.home_dir.join(".codex/config.toml");
    let original = format!("{USER_RELAY}{USER_OTHERS}");
    fs::write(&path, &original).unwrap();
    define_codex(
        &ctx,
        json!({"active": "relay", "profiles": {"relay": {"base_url": "https://relay.example.com/v1", "token_var": "RELAY_KEY"}}}),
    );

    let refused = configure(&ctx, "codex", &["--use", "relay"]);
    assert!(!refused.ok, "追加同名表会生成重复表，必须拒绝");
    assert_eq!(refused.errors[0].code, "AGENT_ERROR");
    assert!(
        refused.errors[0].message.contains("拒绝覆盖")
            && refused.errors[0]
                .message
                .contains("config.toml 中已有你自己的同名接入方式 relay")
            && refused.errors[0].message.contains("换个名字"),
        "{:?}",
        refused.errors
    );
    assert_eq!(codex_config(&ctx), original, "拒绝时一个字节都不许动");
    assert!(!has_codex_binding(&ctx), "拒绝时库里不许记下认领");

    // 接管通道已退役：带上旧标志也不会替换用户的表
    let takeover = configure(&ctx, "codex", &["--use", "relay", "--allow-takeover"]);
    assert!(!takeover.ok);
    assert_eq!(codex_config(&ctx), original);

    // 换个名字：写入成功，用户那张表与子表逐字节保留
    let renamed = configure(
        &ctx,
        "codex",
        &[
            "--use",
            "relay2",
            "--connection",
            r#"{"base_url": "https://relay.example.com/v1", "token_var": "RELAY_KEY"}"#,
        ],
    );
    assert!(renamed.ok, "{:?}", renamed.errors);
    let after = codex_config(&ctx);
    assert!(after.parse::<toml::Value>().is_ok(), "{after}");
    assert!(after.contains(USER_RELAY), "{after}");
    assert!(after.contains(USER_OTHERS), "{after}");
    assert_eq!(
        header_count(&after, "[model_providers.relay]"),
        1,
        "{after}"
    );
    assert_eq!(
        header_count(&after, "[model_providers.relay2]"),
        1,
        "{after}"
    );
    assert_eq!(owned_providers(&ctx), json!(["relay2"]));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn codex_本工具自己写过的同名_provider_再次写入不受影响() {
    let (base, ctx) = temp_ctx("codex-provider-own");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    fs::write(ctx.home_dir.join(".codex/config.toml"), USER_OTHERS).unwrap();
    define_codex(
        &ctx,
        json!({"active": "relay", "profiles": {"relay": {"base_url": "https://relay.example.com/v1"}}}),
    );
    let first = configure(&ctx, "codex", &["--use", "relay"]);
    assert!(first.ok, "{:?}", first.errors);
    assert_eq!(owned_providers(&ctx), json!(["relay"]));

    // 编辑当前那套：写文件并更新库，同一张表原位替换
    let second = configure(
        &ctx,
        "codex",
        &[
            "--use",
            "relay",
            "--connection",
            r#"{"base_url": "https://relay2.example.com/v1"}"#,
        ],
    );
    assert!(second.ok, "{:?}", second.errors);
    let after = codex_config(&ctx);
    assert!(after.parse::<toml::Value>().is_ok(), "{after}");
    assert_eq!(
        header_count(&after, "[model_providers.relay]"),
        1,
        "{after}"
    );
    assert!(after.contains("https://relay2.example.com/v1"), "{after}");
    assert!(after.contains(USER_OTHERS), "{after}");
    assert_eq!(owned_providers(&ctx), json!(["relay"]));

    let _ = fs::remove_dir_all(&base);
}

// ---------- 可用性检查 ----------

#[test]
fn 检查在发请求之前就把缺东西说清楚_且零网络() {
    let (base, ctx) = temp_ctx("check-preflight");

    // 没有接入配置：直说现在用的是助手自带账号
    define_settings(&ctx, json!({}));
    let none = execute(
        &ctx,
        &request("agents:check", &["--only", "agents:claude-code"]),
    );
    assert!(!none.ok);
    assert!(
        none.errors[0].message.contains("助手自带"),
        "{:?}",
        none.errors
    );

    // 缺地址 / 缺模型 / 缺令牌变量名，各自给出能照着做的话
    for (settings, needle) in [
        (json!({"active": "p", "profiles": {"p": {}}}), "没填地址"),
        (
            json!({"active": "p", "profiles": {"p": {"base_url": "https://api.example.com"}}}),
            "没填模型",
        ),
        (
            json!({"active": "p", "profiles": {"p": {"base_url": "https://api.example.com", "model": "m"}}}),
            "没填令牌变量名",
        ),
    ] {
        define_settings(&ctx, settings);
        let env = execute(
            &ctx,
            &request("agents:check", &["--only", "agents:claude-code"]),
        );
        assert!(!env.ok);
        assert!(
            env.errors[0].message.contains(needle),
            "期望提到「{needle}」，实际：{:?}",
            env.errors
        );
    }

    // 变量没取值时也不该发请求
    define_settings(
        &ctx,
        json!({"active": "p", "profiles": {"p": {"base_url": "https://api.example.com", "model": "m", "token_var": "NOPE_TOKEN"}}}),
    );
    let missing = execute(
        &ctx,
        &request("agents:check", &["--only", "agents:claude-code"]),
    );
    assert!(!missing.ok);
    assert!(missing.errors[0].message.contains("NOPE_TOKEN"));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 检查不改配置文件_也不动生效的那套() {
    let (base, ctx) = temp_ctx("check-readonly");
    configure_secret_source(&ctx);
    define_settings(
        &ctx,
        json!({
            "active": "a",
            // 端口 1 会被立即拒绝：这条用例要的是「检查没改任何东西」，
            // 不是真去连一个外网地址干等超时
            "profiles": {
                "a": {"base_url": "https://127.0.0.1:1", "model": "m", "token_var": "T"},
                "b": {"base_url": "https://127.0.0.1:1", "model": "m", "token_var": "T"}
            }
        }),
    );
    assert!(
        execute(
            &ctx,
            &request("secrets:set", &["--var", "T", "--value", "v"])
        )
        .ok
    );
    assert!(configure(&ctx, "claude-code", &["--use", "a"]).ok);
    let before = fs::read(ctx.home_dir.join(".claude/settings.json")).unwrap();

    // --use 测的是另一套，但不该把生效的那套切过去
    let env = execute(
        &ctx,
        &request(
            "agents:check",
            &["--only", "agents:claude-code", "--use", "b"],
        ),
    );
    // 联不联得上取决于环境，这里只断言它没有改任何东西
    assert_eq!(env.data["profile"], json!("b"), "{:?}", env.errors);
    assert_eq!(
        fs::read(ctx.home_dir.join(".claude/settings.json")).unwrap(),
        before,
        "检查是只读的，不许碰配置文件"
    );
    let policy = execute(&ctx, &request("policy:get", &[]));
    assert_eq!(
        policy.data["effective"]["agents"]["definitions"]["claude-code"]["settings"]["active"], "a",
        "检查不该改生效的那套"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 检查引用型_codex_接入_地址与令牌变量取自被引用的表_表不在就明确报错() {
    let (base, ctx) = temp_ctx("check-provider-ref");
    configure_secret_source(&ctx);
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    // 端口 1 会被立即拒绝：这条用例要的是「从表里取到了端点与令牌」，不是真去连
    let path = ctx.home_dir.join(".codex/config.toml");
    fs::write(
        &path,
        "model_provider = \"kimi\"\nmodel = \"k2\"\n\n[model_providers.kimi]\nbase_url = \"https://127.0.0.1:1/v1\"\nenv_key = \"KIMI_KEY\"\n",
    )
    .unwrap();
    define_codex(
        &ctx,
        json!({"profiles": {"imported": {"provider_ref": "kimi", "model": "k2"}}}),
    );
    let before = fs::read(&path).unwrap();

    let missing_var = execute(
        &ctx,
        &request(
            "agents:check",
            &["--only", "agents:codex", "--use", "imported"],
        ),
    );
    assert!(!missing_var.ok);
    assert!(
        missing_var.errors[0].message.contains("KIMI_KEY"),
        "{:?}",
        missing_var.errors
    );
    assert!(!missing_var.errors[0].message.contains("没填地址"));

    assert!(
        execute(
            &ctx,
            &request("secrets:set", &["--var", "KIMI_KEY", "--value", "v"])
        )
        .ok
    );
    let checked = execute(
        &ctx,
        &request(
            "agents:check",
            &["--only", "agents:codex", "--use", "imported"],
        ),
    );
    assert_eq!(checked.data["profile"], "imported", "{:?}", checked.errors);
    assert_eq!(checked.data["endpoint"], "https://127.0.0.1:1/v1/responses");
    assert_eq!(fs::read(&path).unwrap(), before, "检查是只读的");

    // 被引用的表不在了：明确说清楚，而不是「没填地址」
    fs::write(&path, "model = \"k2\"\n").unwrap();
    let gone = execute(
        &ctx,
        &request(
            "agents:check",
            &["--only", "agents:codex", "--use", "imported"],
        ),
    );
    assert!(!gone.ok);
    assert!(
        gone.errors[0].message.contains("已不在 config.toml 中"),
        "{:?}",
        gone.errors
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 检查不存在的接入配置被拒() {
    let (base, ctx) = temp_ctx("check-missing-profile");
    define_settings(
        &ctx,
        json!({"active": "p", "profiles": {"p": {"base_url": "https://a.example.com", "model": "m", "token_var": "T"}}}),
    );
    let env = execute(
        &ctx,
        &request(
            "agents:check",
            &["--only", "agents:claude-code", "--use", "nope"],
        ),
    );
    assert!(!env.ok);
    assert!(env.errors[0].message.contains("nope"));
    let _ = fs::remove_dir_all(&base);
}
