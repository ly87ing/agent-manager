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
        serde_json::to_vec_pretty(
            &json!({"env": {"ANTHROPIC_BASE_URL": "https://mine.example.com"}}),
        )
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
    define_settings(&ctx, json!({"yolo": true}));

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
        &request(
            "secrets:set",
            &["--var", "MY_GATEWAY_TOKEN", "--value", SECRET],
        ),
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

    let refused = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:opencode"]),
    );
    assert!(!refused.ok);
    assert!(!ctx.home_dir.join(".claude/settings.json").exists());
    assert!(!ctx.home_dir.join(".codex/config.toml").exists());

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
    define(
        &ctx,
        json!({"base_url": "https://api.example.com", "model": "m1"}),
    );
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
    define_settings(&ctx, json!({}));
    let cleared = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:claude-code"]),
    );
    assert!(cleared.ok, "{:?}", cleared.errors);

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
    assert!(
        execute(
            &ctx,
            &request("agents:configure", &["--only", "agents:claude-code"])
        )
        .ok
    );
    assert!(
        settings_file(&ctx).get("env").is_none(),
        "官方配置不需要环境变量"
    );

    // 切到中转：一条命令既改库也写文件
    let switched = execute(
        &ctx,
        &request(
            "agents:configure",
            &["--only", "agents:claude-code", "--use", "relay"],
        ),
    );
    assert!(switched.ok, "{:?}", switched.errors);
    assert_eq!(switched.data["active"], "relay");
    let after = settings_file(&ctx);
    assert_eq!(
        after["env"]["ANTHROPIC_BASE_URL"],
        "https://relay.example.com"
    );
    assert_eq!(after["env"]["ANTHROPIC_MODEL"], "relay-model");

    // 库里的 active 也跟着变了，下次读定义不会切回去
    let policy = execute(&ctx, &request("policy:get", &[]));
    assert_eq!(
        policy.data["effective"]["agents"]["definitions"]["claude-code"]["settings"]["active"],
        "relay"
    );

    // 再切回官方：上一套留下的变量一个都不许剩
    let back = execute(
        &ctx,
        &request(
            "agents:configure",
            &["--only", "agents:claude-code", "--use", "official"],
        ),
    );
    assert!(back.ok, "{:?}", back.errors);
    assert!(
        settings_file(&ctx).get("env").is_none(),
        "切回后不得残留上一套的变量"
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
    assert!(
        execute(
            &ctx,
            &request("agents:configure", &["--only", "agents:claude-code"])
        )
        .ok
    );
    let before = fs::read(ctx.home_dir.join(".claude/settings.json")).unwrap();

    let refused = execute(
        &ctx,
        &request(
            "agents:configure",
            &["--only", "agents:claude-code", "--use", "nope"],
        ),
    );
    assert!(!refused.ok);
    assert_eq!(
        fs::read(ctx.home_dir.join(".claude/settings.json")).unwrap(),
        before,
        "被拒时一个字节都不许动"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 内置状态栏会部署脚本并指过去_切回不碰只撤自己那条() {
    let (base, ctx) = temp_ctx("statusline");
    define_settings(&ctx, json!({"statusline": "builtin"}));

    let env = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:claude-code"]),
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["statusline"], true);

    let script = ctx.data_dir.join("statusline.sh");
    assert!(script.is_file(), "脚本要落到数据目录");
    let body = fs::read_to_string(&script).unwrap();
    assert!(body.starts_with("#!/bin/bash"));
    assert!(
        body.contains("agent-manager-usage.json"),
        "落盘是它存在的理由"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&script).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "状态栏脚本要可执行");
    }
    assert_eq!(
        settings_file(&ctx)["statusLine"]["command"],
        script.display().to_string()
    );

    // 切回「不碰」：只撤掉我们指过去的那条
    define_settings(&ctx, json!({}));
    assert!(
        execute(
            &ctx,
            &request("agents:configure", &["--only", "agents:claude-code"])
        )
        .ok
    );
    assert!(settings_file(&ctx).get("statusLine").is_none());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 用户自己的状态栏不被碰_也不算外部修改() {
    let (base, ctx) = temp_ctx("statusline-user");
    // 用户先有自己的状态栏
    fs::write(
        ctx.home_dir.join(".claude/settings.json"),
        serde_json::to_vec_pretty(&json!({
            "statusLine": {"type": "command", "command": "/x/mine.sh"}
        }))
        .unwrap(),
    )
    .unwrap();
    define_settings(
        &ctx,
        json!({"active": "p", "profiles": {"p": {"base_url": "https://api.example.com"}}}),
    );

    // 不该因为他有自己的状态栏就判成冲突
    let env = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:claude-code"]),
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

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 两种认证方式各写各的环境变量() {
    let (base, ctx) = temp_ctx("auth");
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
    assert!(
        execute(
            &ctx,
            &request("agents:configure", &["--only", "agents:claude-code"])
        )
        .ok
    );
    let after = settings_file(&ctx);
    assert_eq!(after["env"]["ANTHROPIC_API_KEY"], SECRET);
    assert!(after["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());

    // 换成 Bearer：上一个变量要消失，否则两个头一起发出去
    define_settings(
        &ctx,
        json!({"active": "p", "profiles": {"p": {"token_var": "RELAY_KEY", "auth": "token"}}}),
    );
    assert!(
        execute(
            &ctx,
            &request("agents:configure", &["--only", "agents:claude-code"])
        )
        .ok
    );
    let after = settings_file(&ctx);
    assert_eq!(after["env"]["ANTHROPIC_AUTH_TOKEN"], SECRET);
    assert!(after["env"].get("ANTHROPIC_API_KEY").is_none());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 切回默认会撤掉接入配置_库里的生效项也清空() {
    let (base, ctx) = temp_ctx("use-default");
    define_settings(
        &ctx,
        json!({
            "active": "relay",
            "profiles": {"relay": {"base_url": "https://relay.example.com"}}
        }),
    );
    assert!(
        execute(
            &ctx,
            &request("agents:configure", &["--only", "agents:claude-code"])
        )
        .ok
    );
    assert_eq!(
        settings_file(&ctx)["env"]["ANTHROPIC_BASE_URL"],
        "https://relay.example.com"
    );

    let back = execute(
        &ctx,
        &request(
            "agents:configure",
            &["--only", "agents:claude-code", "--use-default"],
        ),
    );
    assert!(back.ok, "{:?}", back.errors);
    assert_eq!(back.data["active"], Value::Null);
    assert!(
        settings_file(&ctx).get("env").is_none(),
        "切回默认后不留任何接入变量"
    );

    // 库里也清了，下次读定义不会又切回中转
    let policy = execute(&ctx, &request("policy:get", &[]));
    assert!(
        policy.data["effective"]["agents"]["definitions"]["claude-code"]["settings"]
            .get("active")
            .is_none()
    );

    // 两个切换参数不能同时给
    let both = execute(
        &ctx,
        &request(
            "agents:configure",
            &[
                "--only",
                "agents:claude-code",
                "--use",
                "relay",
                "--use-default",
            ],
        ),
    );
    assert!(!both.ok);

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

    let env = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:codex"]),
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
    assert!(
        execute(
            &ctx,
            &request("agents:configure", &["--only", "agents:codex"])
        )
        .ok
    );
    assert!(codex_config(&ctx).contains("[model_providers.a]"));

    let switched = execute(
        &ctx,
        &request(
            "agents:configure",
            &["--only", "agents:codex", "--use", "b"],
        ),
    );
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

    let back = execute(
        &ctx,
        &request(
            "agents:configure",
            &["--only", "agents:codex", "--use-default"],
        ),
    );
    assert!(back.ok, "{:?}", back.errors);
    let after = codex_config(&ctx);
    assert!(
        !after.contains("model_providers"),
        "切回默认后不留接入方式：{after}"
    );
    assert!(!after.contains("model_provider ="));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn codex_跳过权限确认写审批与沙箱_关掉则撤回() {
    let (base, ctx) = temp_ctx("codex-yolo");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    define_codex(&ctx, json!({"yolo": true}));

    let refused = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:codex"]),
    );
    assert!(!refused.ok, "降安全等级必须显式确认");

    let ok = execute(
        &ctx,
        &request(
            "agents:configure",
            &["--only", "agents:codex", "--allow-yolo"],
        ),
    );
    assert!(ok.ok, "{:?}", ok.errors);
    let after = codex_config(&ctx);
    assert!(after.contains("approval_policy = \"never\""));
    assert!(after.contains("sandbox_mode = \"danger-full-access\""));

    // 关掉：我们写的那两项撤回
    define_codex(&ctx, json!({}));
    assert!(
        execute(
            &ctx,
            &request("agents:configure", &["--only", "agents:codex"])
        )
        .ok
    );
    let after = codex_config(&ctx);
    assert!(!after.contains("approval_policy"));
    assert!(!after.contains("sandbox_mode"));

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
    let refused = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:codex"]),
    );
    assert!(!refused.ok, "内置名字不该放行");

    // 自定义接入方式没有默认地址，只填令牌变量名不成立
    define_codex(
        &ctx,
        json!({"active": "relay", "profiles": {"relay": {"token_var": "RELAY_KEY"}}}),
    );
    let refused = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:codex"]),
    );
    assert!(!refused.ok, "缺地址要当场说清楚");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn codex_外部改过我们那几项就拒绝覆盖() {
    let (base, ctx) = temp_ctx("codex-takeover");
    fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    define_codex(
        &ctx,
        json!({"active": "relay", "profiles": {"relay": {"base_url": "https://relay.example.com/v1"}}}),
    );
    assert!(
        execute(
            &ctx,
            &request("agents:configure", &["--only", "agents:codex"])
        )
        .ok
    );

    // 本工具以外把地址改掉
    let path = ctx.home_dir.join(".codex/config.toml");
    let edited = codex_config(&ctx).replace(
        "https://relay.example.com/v1",
        "https://hand.example.com/v1",
    );
    fs::write(&path, &edited).unwrap();

    let refused = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:codex"]),
    );
    assert!(!refused.ok, "外部改过就该拒绝");
    assert_eq!(codex_config(&ctx), edited, "拒绝时一个字节都不许动");

    let taken = execute(
        &ctx,
        &request(
            "agents:configure",
            &["--only", "agents:codex", "--allow-takeover"],
        ),
    );
    assert!(taken.ok, "{:?}", taken.errors);
    assert!(codex_config(&ctx).contains("https://relay.example.com/v1"));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn codex_用户自己配的顶层键首次也要显式接管() {
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

    let refused = execute(
        &ctx,
        &request("agents:configure", &["--only", "agents:codex"]),
    );
    assert!(!refused.ok, "不问自取会把用户手写的顶掉");
    assert!(codex_config(&ctx).contains("gpt-5.6-sol"));

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
    assert!(
        execute(
            &ctx,
            &request("agents:configure", &["--only", "agents:claude-code"])
        )
        .ok
    );
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
