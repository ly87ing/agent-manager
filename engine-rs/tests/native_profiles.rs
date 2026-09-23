//! profiles 域：实例定义、只读状态、真实写入与安全反例。
//!
//! 核心保证：全局 ~/.claude 与 ~/.codex 全程不被触碰；鉴权取值只以引用形式入库，
//! Claude 目标文件写解析后的值（客户端不展开引用），Codex 目标只写变量名。

use agent_manager_engine::{execute, store, Context, Request};
use std::path::PathBuf;

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    let base =
        std::env::temp_dir().join(format!("am-profiles-test-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&base);
    let ctx = Context {
        data_dir: base.join("data"),
        home_dir: base.join("home"),
    };
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    (base, ctx)
}

fn request(command: &str, args: &[&str]) -> Request {
    Request {
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        run_id: "profiles-test".to_string(),
    }
}

fn save_definition(ctx: &Context, id: &str, definition: &str) -> bool {
    execute(
        ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                &format!("agents.definitions.{id}"),
                "--value",
                definition,
            ],
        ),
    )
    .ok
}

fn element<'a>(env: &'a agent_manager_engine::Envelope, id: &str) -> &'a serde_json::Value {
    env.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|el| el["id"] == format!("profiles:{id}"))
        .unwrap_or_else(|| panic!("缺少实例 {id}"))
}

/// 测试并行执行，环境变量是进程级共享的：每个用例用各自的变量名，互不清掉对方的。
fn claude_profile(token_var: &str) -> String {
    format!(
        r#"{{"display_name":"Claude Code · ly","bin":"claude","skills_dir":"~/.claude-ly/skills","mcp_config":{{"path":"~/.claude-ly/.claude.json","format":"json"}},"profile":{{"base_agent":"claude-code","config_dir":"~/.claude-ly","env":{{"ANTHROPIC_BASE_URL":"https://api.example.com","ANTHROPIC_AUTH_TOKEN":"${{{token_var}}}"}}}},"_delete":false,"_replace":true}}"#
    )
}

#[test]
fn claude_profile_writes_isolated_settings_without_touching_global() {
    let (root, ctx) = temp_ctx("claude-write");
    // 主实例配置：全程不得被改
    std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
    let global = ctx.home_dir.join(".claude/settings.json");
    std::fs::write(&global, "{\"hooks\":{}}\n").unwrap();
    let global_before = std::fs::read(&global).unwrap();

    std::env::set_var("AM_TEST_TOKEN_WRITE", "sentinel-token-value");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_TOKEN_WRITE")
    ));

    // status 只读零写：不建配置目录、不写目标文件
    let status = execute(&ctx, &request("profiles:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    let el = element(&status, "cc-ly");
    assert_eq!(el["state"], "not-materialized");
    assert_eq!(el["observed"]["config_dir"], "~/.claude-ly");
    assert_eq!(el["observed"]["config_env"], "CLAUDE_CONFIG_DIR");
    assert_eq!(
        el["observed"]["launch_value"],
        "CLAUDE_CONFIG_DIR=\"~/.claude-ly\" claude"
    );
    assert!(
        !ctx.home_dir.join(".claude-ly").exists(),
        "status 不得创建目录"
    );
    // 状态只报变量名与是否可解析，不含取值
    let refs = el["observed"]["env_refs"].as_array().unwrap();
    let token_ref = refs
        .iter()
        .find(|r| r["name"] == "ANTHROPIC_AUTH_TOKEN")
        .unwrap();
    assert_eq!(token_ref["kind"], "reference");
    assert_eq!(token_ref["resolved"], true);
    assert!(!serde_json::to_string(&status.data)
        .unwrap()
        .contains("sentinel-token-value"));

    // 真实写入
    let apply = execute(
        &ctx,
        &request("profiles:apply", &["--only", "profiles:cc-ly"]),
    );
    assert!(apply.ok, "{:?}", apply.errors);
    let target = ctx.home_dir.join(".claude-ly/settings.json");
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
    // Claude 不展开引用，目标必须是解析后的真实值（与 MCP stdio env 同语义）
    assert_eq!(
        written["env"]["ANTHROPIC_AUTH_TOKEN"],
        "sentinel-token-value"
    );
    assert_eq!(
        written["env"]["ANTHROPIC_BASE_URL"],
        "https://api.example.com"
    );

    // 全局配置一个字节都没动
    assert_eq!(std::fs::read(&global).unwrap(), global_before);

    // 库里零明文
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let policy_raw: String = conn
        .query_row(
            "SELECT json FROM policy_layers WHERE layer='local'",
            [],
            |r| r.get(0),
        )
        .unwrap_or_default();
    assert!(
        !policy_raw.contains("sentinel-token-value"),
        "库里不得出现取值"
    );
    drop(conn);

    // 写入后状态转为已就绪
    let after = execute(&ctx, &request("profiles:status", &[]));
    assert_eq!(element(&after, "cc-ly")["state"], "managed-ok");

    std::env::remove_var("AM_TEST_TOKEN_WRITE");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn codex_profile_target_never_contains_the_secret_value() {
    let (root, ctx) = temp_ctx("codex-write");
    std::env::set_var("AM_TEST_CODEX_TOKEN", "codex-sentinel-value");
    assert!(save_definition(
        &ctx,
        "cx-work",
        r#"{"display_name":"Codex · work","bin":"codex","profile":{"base_agent":"codex","config_dir":"~/.codex-work","base_url":"https://api.example.com/v1","env_key":"AM_TEST_CODEX_TOKEN","model":"gpt-5.5"},"_delete":false,"_replace":true}"#
    ));
    let apply = execute(
        &ctx,
        &request("profiles:apply", &["--only", "profiles:cx-work"]),
    );
    assert!(apply.ok, "{:?}", apply.errors);
    // 供应商配置写独立的叠加文件，config.toml 留给 MCP 域，两者互不覆盖
    assert!(
        !ctx.home_dir.join(".codex-work/config.toml").exists(),
        "不得写 config.toml（那是 MCP 域的目标）"
    );
    let text = std::fs::read_to_string(ctx.home_dir.join(".codex-work/agent-manager.config.toml"))
        .unwrap();
    assert!(text.contains("env_key = \"AM_TEST_CODEX_TOKEN\""));
    assert!(text.contains("model_provider = \"agent-manager\""));
    let status = execute(&ctx, &request("profiles:status", &[]));
    assert_eq!(
        element(&status, "cx-work")["observed"]["launch_value"],
        "CODEX_HOME=\"~/.codex-work\" codex -p agent-manager"
    );
    assert!(
        !text.contains("codex-sentinel-value"),
        "Codex 目标只写变量名，绝不写取值"
    );
    std::env::remove_var("AM_TEST_CODEX_TOKEN");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unsafe_config_dirs_are_rejected_before_any_write() {
    let (root, ctx) = temp_ctx("guards");
    for (id, dir) in [
        ("cc-primary", "~/.claude"),
        ("cc-home", "~"),
        ("cc-escape", "~/../outside"),
        ("cc-abs", "/etc/agent-manager"),
    ] {
        let definition = format!(
            r#"{{"bin":"claude","profile":{{"base_agent":"claude-code","config_dir":"{dir}"}},"_delete":false,"_replace":true}}"#
        );
        assert!(
            save_definition(&ctx, id, &definition),
            "保存定义本身不执行动作"
        );
        let apply = execute(
            &ctx,
            &request("profiles:apply", &["--only", &format!("profiles:{id}")]),
        );
        assert!(!apply.ok, "[{dir}] 必须 fail-closed");
        assert_eq!(apply.errors[0].code, "PROFILE_ERROR");
    }
    // 主实例目录没有被创建或写入
    assert!(!ctx.home_dir.join(".claude/settings.json").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn existing_foreign_target_needs_explicit_takeover() {
    let (root, ctx) = temp_ctx("takeover");
    std::env::set_var("AM_TEST_TOKEN_TAKEOVER", "x");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_TOKEN_TAKEOVER")
    ));
    // 用户自己先放了一份 settings
    std::fs::create_dir_all(ctx.home_dir.join(".claude-ly")).unwrap();
    let target = ctx.home_dir.join(".claude-ly/settings.json");
    std::fs::write(&target, "{\"env\":{\"MINE\":\"keep\"}}\n").unwrap();

    let denied = execute(
        &ctx,
        &request("profiles:apply", &["--only", "profiles:cc-ly"]),
    );
    assert!(!denied.ok, "已有他人内容时必须拒绝覆盖");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "{\"env\":{\"MINE\":\"keep\"}}\n",
        "拒绝时原文件不得改动"
    );

    let allowed = execute(
        &ctx,
        &request(
            "profiles:apply",
            &["--only", "profiles:cc-ly", "--allow-takeover"],
        ),
    );
    assert!(allowed.ok, "{:?}", allowed.errors);
    assert!(std::fs::read_to_string(&target)
        .unwrap()
        .contains("ANTHROPIC_BASE_URL"));

    // 写入后被外部改动 → 状态报漂移，再次写入又需要接管
    std::fs::write(&target, "{\"env\":{\"EXTERNAL\":\"edit\"}}\n").unwrap();
    let status = execute(&ctx, &request("profiles:status", &[]));
    assert_eq!(element(&status, "cc-ly")["state"], "drifted");
    let denied2 = execute(
        &ctx,
        &request("profiles:apply", &["--only", "profiles:cc-ly"]),
    );
    assert!(!denied2.ok, "外部改过之后必须再次要求接管");

    std::env::remove_var("AM_TEST_TOKEN_TAKEOVER");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn missing_variable_and_bad_definitions_fail_closed() {
    let (root, ctx) = temp_ctx("invalid");
    // 缺变量：拒绝写出不完整配置
    assert!(save_definition(
        &ctx,
        "cc-miss",
        r#"{"bin":"claude","profile":{"base_agent":"claude-code","config_dir":"~/.claude-miss","env":{"ANTHROPIC_AUTH_TOKEN":"${AM_TEST_ABSENT_VAR}"}},"_delete":false,"_replace":true}"#
    ));
    let apply = execute(
        &ctx,
        &request("profiles:apply", &["--only", "profiles:cc-miss"]),
    );
    assert!(!apply.ok);
    assert!(
        !ctx.home_dir.join(".claude-miss").exists(),
        "预检失败不得建目录"
    );

    // 鉴权取值写字面量：定义解析即拒绝，status 用固定语义块呈现
    assert!(save_definition(
        &ctx,
        "cc-plain",
        r#"{"bin":"claude","profile":{"base_agent":"claude-code","config_dir":"~/.claude-plain","env":{"ANTHROPIC_AUTH_TOKEN":"literal-secret"}},"_delete":false,"_replace":true}"#
    ));
    let status = execute(&ctx, &request("profiles:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    let text = serde_json::to_string(&status.data).unwrap();
    assert!(text.contains("unsupported"), "非法定义必须以固定状态呈现");
    assert!(!text.contains("literal-secret"), "状态不得回显取值");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn status_is_read_only_and_args_are_strict() {
    let (root, ctx) = temp_ctx("readonly");
    let env = execute(&ctx, &request("profiles:status", &[]));
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["elements"].as_array().unwrap().len(), 0);
    assert!(!ctx.data_dir.exists(), "缺库读取不得创建数据目录");

    for args in [&["--bogus"][..], &["--dry-run"][..]] {
        let bad = execute(&ctx, &request("profiles:status", args));
        assert!(!bad.ok, "参数 {:?} 必须被拒绝", args);
    }
    let bad_apply = execute(
        &ctx,
        &request("profiles:apply", &["--only", "profiles:cc-ly", "--dry-run"]),
    );
    assert!(!bad_apply.ok);
    let _ = std::fs::remove_dir_all(root);
}

/// 实例目录下由本工具写的是配置与 Hooks；规则文件以实际文件为准，归规则页管理，
/// profiles:apply 既不生成也不检测它。主实例的固定路径一个字节都不动。
#[test]
fn profile_directory_gets_hooks_but_leaves_rules_file_alone() {
    let (root, ctx) = temp_ctx("full-set");
    std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
    let global_settings = ctx.home_dir.join(".claude/settings.json");
    let global_rules = ctx.home_dir.join(".claude/CLAUDE.md");
    std::fs::write(&global_settings, "{\"theme\":\"dark\"}\n").unwrap();
    std::fs::write(&global_rules, "# 主实例规则\n").unwrap();
    let settings_before = std::fs::read(&global_settings).unwrap();
    let rules_before = std::fs::read(&global_rules).unwrap();

    std::env::set_var("AM_TEST_TOKEN_FULLSET", "x");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_TOKEN_FULLSET")
    ));
    // 用户自己在实例里加过的设置：合并写入必须保留
    std::fs::create_dir_all(ctx.home_dir.join(".claude-ly")).unwrap();
    std::fs::write(
        ctx.home_dir.join(".claude-ly/settings.json"),
        "{\"statusLine\":\"mine\"}\n",
    )
    .unwrap();

    let apply = execute(
        &ctx,
        &request(
            "profiles:apply",
            &["--only", "profiles:cc-ly", "--allow-takeover"],
        ),
    );
    assert!(apply.ok, "{:?}", apply.errors);

    // 1) settings.json：本工具管的字段写入，用户字段保留
    let settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(ctx.home_dir.join(".claude-ly/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(settings["statusLine"], "mine", "用户自己的设置不得被抹掉");
    assert!(settings["env"]["ANTHROPIC_BASE_URL"].is_string());
    assert!(settings["hooks"].is_object(), "Hooks 也随实例落地");

    // 2) 新建实例不自动写规则文件（用户在规则页从主助手复制或自己写）
    let instance_rules = ctx.home_dir.join(".claude-ly/CLAUDE.md");
    assert!(!instance_rules.exists(), "profiles:apply 不再生成规则文件");

    // 3) 主实例的固定路径全程未被触碰
    assert_eq!(std::fs::read(&global_settings).unwrap(), settings_before);
    assert_eq!(std::fs::read(&global_rules).unwrap(), rules_before);

    // 4) 用户自己写的实例规则：状态不因它变化，再次应用也不碰它的字节
    std::fs::write(&instance_rules, "external edit\n").unwrap();
    let status = execute(&ctx, &request("profiles:status", &[]));
    assert_eq!(element(&status, "cc-ly")["state"], "managed-ok");
    let again = execute(
        &ctx,
        &request("profiles:apply", &["--only", "profiles:cc-ly"]),
    );
    assert!(again.ok, "{:?}", again.errors);
    assert_eq!(
        std::fs::read_to_string(&instance_rules).unwrap(),
        "external edit\n",
        "实例规则文件字节不变"
    );

    std::env::remove_var("AM_TEST_TOKEN_FULLSET");
    let _ = std::fs::remove_dir_all(root);
}

/// Codex 实例同样拿到 Hooks，不生成规则文件，也不碰归 MCP 域的 config.toml。
#[test]
fn codex_profile_directory_gets_hooks_without_rules_file() {
    let (root, ctx) = temp_ctx("codex-full-set");
    assert!(save_definition(
        &ctx,
        "cx-work",
        r#"{"display_name":"Codex · work","bin":"codex","profile":{"base_agent":"codex","config_dir":"~/.codex-work","base_url":"https://api.example.com/v1","env_key":"AM_TEST_ABSENT_OK","model":"gpt-5.5"},"_delete":false,"_replace":true}"#
    ));
    let apply = execute(
        &ctx,
        &request("profiles:apply", &["--only", "profiles:cx-work"]),
    );
    assert!(apply.ok, "{:?}", apply.errors);

    let dir = ctx.home_dir.join(".codex-work");
    assert!(dir.join("agent-manager.config.toml").is_file());
    assert!(!dir.join("config.toml").exists(), "config.toml 仍归 MCP 域");
    assert!(!dir.join("AGENTS.md").exists(), "规则文件不由实例域生成");
    let hooks: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("hooks.json")).unwrap()).unwrap();
    assert!(hooks["hooks"].is_object());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn 实例的额度读它自己的配置目录_不是默认那份() {
    let (root, ctx) = temp_ctx("usage");
    // 默认位置放一份「别人的」额度：读串了就会看到 99
    std::fs::write(
        ctx.home_dir.join(".claude.json"),
        r#"{"cachedUsageUtilization":{"fetchedAtMs":1789605697689,"utilization":{"five_hour":{"utilization":99,"resets_at":null}}}}"#,
    )
    .unwrap();
    // 实例自己的配置目录
    let instance = ctx.home_dir.join(".claude-ly");
    std::fs::create_dir_all(&instance).unwrap();
    std::fs::write(
        instance.join(".claude.json"),
        r#"{"cachedUsageUtilization":{"fetchedAtMs":1789605697689,"utilization":{"five_hour":{"utilization":7,"resets_at":null}}}}"#,
    )
    .unwrap();

    std::env::set_var("AM_TEST_TOKEN_USAGE", "t");
    assert!(save_definition(
        &ctx,
        "cc-usage",
        &claude_profile("AM_TEST_TOKEN_USAGE")
    ));

    let env = execute(&ctx, &request("profiles:status", &[]));
    assert!(env.ok, "profiles:status 失败: {:?}", env.errors);
    let usage = &element(&env, "cc-usage")["observed"]["usage"];
    assert_eq!(usage["windows"][0]["used_percent"], serde_json::json!(7.0));
    assert_eq!(
        usage["source"],
        serde_json::json!(instance.join(".claude.json").display().to_string())
    );

    std::env::remove_var("AM_TEST_TOKEN_USAGE");
    let _ = std::fs::remove_dir_all(&root);
}
