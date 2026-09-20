//! 助手运行参数落到 `~/.claude/settings.json`：只改本工具拥有的那几条路径，
//! 外部改过就拒绝覆盖，YOLO 少了显式确认标志一律拒绝。

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

use agent_manager_engine::{execute, store, Context, Request};

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

/// 写一条带 settings 段的助手定义（内置 claude-code 的覆盖）。
fn define(ctx: &Context, settings: Value) {
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

#[test]
fn 写入只动自己那几条路径_别人的键原样保留() {
    let (base, ctx) = temp_ctx("merge");
    // 用户自己的设置先在那儿：hooks 归 hooks 域，statusLine 和别的 env 是用户的
    fs::write(
        ctx.home_dir.join(".claude/settings.json"),
        serde_json::to_vec_pretty(&json!({
            "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "x"}]}]},
            "statusLine": {"type": "command", "command": "/x/statusline.sh"},
            "env": {"MY_OWN": "keep-me"}
        }))
        .unwrap(),
    )
    .unwrap();
    define(
        &ctx,
        json!({"base_url": "https://api.example.com", "model": "claude-opus-5"}),
    );

    let env = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:claude-code"]),
    );
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.lock.held, "写动作要取排他锁");

    let after = settings_file(&ctx);
    assert!(after["hooks"]["SessionStart"].is_array(), "hooks 域的内容不能动");
    assert_eq!(after["statusLine"]["command"], "/x/statusline.sh");
    assert_eq!(after["env"]["MY_OWN"], "keep-me");
    assert_eq!(after["env"]["ANTHROPIC_BASE_URL"], "https://api.example.com");
    assert_eq!(after["model"], "claude-opus-5");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 外部改过我们那几项就拒绝覆盖_接管标志放行() {
    let (base, ctx) = temp_ctx("takeover");
    define(&ctx, json!({"base_url": "https://api.example.com"}));
    let ok = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:claude-code"]),
    );
    assert!(ok.ok, "{:?}", ok.errors);

    // 本工具以外把地址改掉
    let path = ctx.home_dir.join(".claude/settings.json");
    let mut current: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    current["env"]["ANTHROPIC_BASE_URL"] = json!("https://hand-edited.example.com");
    fs::write(&path, serde_json::to_vec_pretty(&current).unwrap()).unwrap();

    let refused = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:claude-code"]),
    );
    assert!(!refused.ok, "外部改过就该拒绝");
    assert_eq!(refused.errors[0].code, "AGENT_ERROR");
    assert_eq!(
        settings_file(&ctx)["env"]["ANTHROPIC_BASE_URL"],
        "https://hand-edited.example.com",
        "拒绝时一个字节都不许动"
    );

    let taken = execute(
        &ctx,
        &request(
            "agents:configure",
            &["--only", "agents:claude-code", "--allow-takeover"],
        ),
    );
    assert!(taken.ok, "{:?}", taken.errors);
    assert_eq!(
        settings_file(&ctx)["env"]["ANTHROPIC_BASE_URL"],
        "https://api.example.com"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 首次写入遇到用户手配的同名键_同样要显式接管() {
    let (base, ctx) = temp_ctx("first-run-existing");
    // 从没用本工具写过，但用户自己配了地址
    fs::write(
        ctx.home_dir.join(".claude/settings.json"),
        serde_json::to_vec_pretty(&json!({"env": {"ANTHROPIC_BASE_URL": "https://mine.example.com"}}))
            .unwrap(),
    )
    .unwrap();
    define(&ctx, json!({"base_url": "https://api.example.com"}));

    let refused = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:claude-code"]),
    );
    assert!(!refused.ok, "不问自取会把用户手写的顶掉");
    assert_eq!(
        settings_file(&ctx)["env"]["ANTHROPIC_BASE_URL"],
        "https://mine.example.com"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn yolo_没有显式确认标志一律拒绝() {
    let (base, ctx) = temp_ctx("yolo");
    define(&ctx, json!({"yolo": true}));

    let refused = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:claude-code"]),
    );
    assert!(!refused.ok, "降安全等级必须显式确认");
    assert!(
        !ctx.home_dir.join(".claude/settings.json").exists(),
        "拒绝时不得落地任何内容"
    );

    let confirmed = execute(
        &ctx,
        &request(
            "agents:configure",
            &["--only", "agents:claude-code", "--allow-yolo"],
        ),
    );
    assert!(confirmed.ok, "{:?}", confirmed.errors);
    assert_eq!(
        settings_file(&ctx)["permissions"]["defaultMode"],
        "bypassPermissions"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 令牌只存变量名_取不到值时拒绝写入_审计不记取值() {
    let (base, ctx) = temp_ctx("token");
    define(&ctx, json!({"token_var": "MY_GATEWAY_TOKEN"}));

    let refused = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:claude-code"]),
    );
    assert!(!refused.ok, "变量没取值就写上去，助手会以为配好了");
    assert!(refused.errors[0].message.contains("MY_GATEWAY_TOKEN"));

    // 在密钥页填上取值
    const SECRET: &str = "sk-gateway-secret-value";
    let set = execute(
        &ctx,
        &request("secrets:set", &["--var", "MY_GATEWAY_TOKEN", "--value", SECRET]),
    );
    assert!(set.ok, "{:?}", set.errors);

    let ok = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:claude-code"]),
    );
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

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 非_claude_形态的助手拒绝写运行参数_且零写() {
    let (base, ctx) = temp_ctx("kind");
    let definition = json!({
        "display_name": "Codex",
        "bin": "codex",
        "preferred_provider": "native",
        "providers": {"native": {"install": "true", "managed": true}},
        "settings": {"base_url": "https://api.example.com"},
    });
    let env = execute(
        &ctx,
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

    let refused = execute(&ctx, &request("agents:configure", &["--only", "agents:codex"]));
    assert!(!refused.ok);
    assert!(!ctx.home_dir.join(".claude/settings.json").exists());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 非法地址与变量名在写之前就被挡住() {
    let (base, ctx) = temp_ctx("validate");
    for bad in [
        json!({"base_url": "http://evil.example.com"}),
        json!({"token_var": "has-dash"}),
    ] {
        define(&ctx, bad);
        let refused = execute(
            &ctx,
            &request("agents:configure", &["--only", "agents:claude-code"]),
        );
        assert!(!refused.ok, "非法输入不该落地");
        assert!(!ctx.home_dir.join(".claude/settings.json").exists());
    }
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 清空参数会把自己那几条撤掉_不碰别人的() {
    let (base, ctx) = temp_ctx("clear");
    define(&ctx, json!({"base_url": "https://api.example.com", "model": "m1"}));
    assert!(
        execute(
            &ctx,
            &request("agents:configure", &["--only", "agents:claude-code"])
        )
        .ok
    );

    // 用户随后往里加了自己的东西
    let path = ctx.home_dir.join(".claude/settings.json");
    let mut current: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    current["statusLine"] = json!({"type": "command", "command": "/x/s.sh"});
    current["env"]["MY_OWN"] = json!("keep-me");
    fs::write(&path, serde_json::to_vec_pretty(&current).unwrap()).unwrap();

    // 把参数清空后再写：只撤自己那几条
    define(&ctx, json!({}));
    let cleared = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:claude-code"]),
    );
    assert!(cleared.ok, "{:?}", cleared.errors);

    let after = settings_file(&ctx);
    assert!(after["env"].get("ANTHROPIC_BASE_URL").is_none());
    assert!(after.get("model").is_none());
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
