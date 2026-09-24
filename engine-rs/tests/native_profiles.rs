//! profiles 域：实例定义、只读状态、真实写入与安全反例。
//!
//! 核心保证：全局 ~/.claude 与 ~/.codex 全程不被触碰；鉴权取值只以引用形式入库，
//! 实例目录里的目标文件写解析后的值（助手不展开引用，启动时也不依赖 shell 导出变量）。

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
    install_fake_clis(&ctx);
    (base, ctx)
}

/// 官方 CLI 的替身，放在 `<HOME>/.local/bin`（探测优先找这里），不依赖本机装没装 claude/codex。
/// 目录里有 `.fake-logged-in` 就报告已登录、有 `.fake-console` 就报告 Console 登录；每次调用把
/// 收到的配置目录与凭据变量是否被清掉记进 `<HOME>/.probe-log`，把 `--settings` 与档案目录记进
/// `<HOME>/.probe-args`。
const FAKE_CLAUDE: &str = r#"#!/bin/sh
settings=none
if [ "$1" = --settings ]; then settings="$2"; shift 2; fi
[ "$1" = auth ] && [ "$2" = status ] || exit 64
printf 'claude|%s|%s\n' "$CLAUDE_CONFIG_DIR" "${CLAUDE_CODE_OAUTH_TOKEN-unset}" >> "$HOME/.probe-log"
printf '%s|%s|%s\n' "$settings" "${ANTHROPIC_CONFIG_DIR-unset}" "${ANTHROPIC_PROFILE-unset}" >> "$HOME/.probe-args"
if [ -f "$HOME/.fake-garbage" ]; then echo 'not json'; exit 0; fi
if [ -f "$CLAUDE_CONFIG_DIR/.fake-logged-in" ]; then echo '{"loggedIn":true,"authMethod":"claude.ai"}'; exit 0; fi
if [ -f "$CLAUDE_CONFIG_DIR/.fake-console" ]; then echo '{"loggedIn":true,"authMethod":"oauth_token","apiKeySource":"/login managed key"}'; exit 0; fi
if [ -f "$CLAUDE_CONFIG_DIR/.fake-helper" ]; then echo '{"loggedIn":true,"authMethod":"api_key_helper","apiKeySource":"apiKeyHelper"}'; exit 0; fi
echo '{"loggedIn":false,"authMethod":"none"}'
exit 1
"#;
const FAKE_CODEX: &str = r#"#!/bin/sh
[ "$1" = login ] && [ "$2" = status ] || exit 64
printf 'codex|%s|%s\n' "$CODEX_HOME" "${CODEX_API_KEY-unset}" >> "$HOME/.probe-log"
if [ -f "$CODEX_HOME/.fake-logged-in" ]; then echo 'Logged in using ChatGPT' >&2; exit 0; fi
echo 'Not logged in' >&2
exit 1
"#;

fn install_fake_clis(ctx: &Context) {
    use std::os::unix::fs::PermissionsExt;
    let bin = ctx.home_dir.join(".local/bin");
    std::fs::create_dir_all(&bin).unwrap();
    for (name, body) in [("claude", FAKE_CLAUDE), ("codex", FAKE_CODEX)] {
        let path = bin.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn probe_log(ctx: &Context) -> String {
    std::fs::read_to_string(ctx.home_dir.join(".probe-log")).unwrap_or_default()
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

fn status(ctx: &Context) -> agent_manager_engine::Envelope {
    let env = execute(ctx, &request("profiles:status", &[]));
    assert!(env.ok, "{:?}", env.errors);
    env
}

/// status 给出的写入基准（实际文件为准）。
fn base_of(ctx: &Context, id: &str) -> String {
    element(&status(ctx), id)["observed"]["base"]
        .as_str()
        .unwrap_or_else(|| panic!("{id} 没有 base"))
        .to_string()
}

/// 按当前实际内容写入实例（先取 base 再 apply）。
fn apply_now(ctx: &Context, id: &str) -> agent_manager_engine::Envelope {
    let base = base_of(ctx, id);
    execute(
        ctx,
        &request(
            "profiles:apply",
            &["--only", &format!("profiles:{id}"), "--base", &base],
        ),
    )
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

    // 没带 base 的写入被拒（必须基于读到的实际内容）
    let no_base = execute(
        &ctx,
        &request("profiles:apply", &["--only", "profiles:cc-ly"]),
    );
    assert!(!no_base.ok);
    assert_eq!(el["observed"]["base"], "missing");

    // 真实写入
    let apply = apply_now(&ctx, "cc-ly");
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

/// 与 Claude 实例同一做法：凭据取值只写进实例自己的 config.toml（0600），库里只有变量名。
/// 此前只写 `env_key`，Codex 要从启动进程的环境变量取值，别名不导出它就报缺变量。
#[test]
fn codex_实例把凭据写进自己的_config_toml_保留_mcp_条目_库里只有变量名() {
    let (root, ctx) = temp_ctx("codex-write");
    std::env::set_var("AM_TEST_CODEX_TOKEN", "codex-sentinel-value");
    let dir = ctx.home_dir.join(".codex-work");
    std::fs::create_dir_all(&dir).unwrap();
    let mcp = "[mcp_servers.fs]\ncommand = \"npx\"\n";
    std::fs::write(dir.join("config.toml"), mcp).unwrap();
    assert!(save_definition(
        &ctx,
        "cx-work",
        r#"{"display_name":"Codex · work","bin":"codex","profile":{"base_agent":"codex","config_dir":"~/.codex-work","base_url":"https://api.example.com/v1","env_key":"AM_TEST_CODEX_TOKEN","model":"gpt-5.5"},"_delete":false,"_replace":true}"#
    ));
    let apply = apply_now(&ctx, "cx-work");
    assert!(apply.ok, "{:?}", apply.errors);
    let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
    let doc: toml::Table = text.parse().unwrap();
    assert_eq!(doc["model_provider"].as_str(), Some("agent-manager"));
    let provider = &doc["model_providers"]["agent-manager"];
    assert_eq!(
        provider["experimental_bearer_token"].as_str(),
        Some("codex-sentinel-value")
    );
    assert!(provider.get("env_key").is_none());
    assert_eq!(
        doc["mcp_servers"]["fs"]["command"].as_str(),
        Some("npx"),
        "MCP 条目原样保留"
    );
    assert!(
        !dir.join("agent-manager.config.toml").exists(),
        "不再写 -p 叠加文件"
    );
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join("config.toml"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "含凭据的文件只有本人可读写");
    }
    let snapshot = status(&ctx);
    let el = element(&snapshot, "cx-work");
    assert_eq!(
        el["observed"]["launch_value"],
        "CODEX_API_KEY= CODEX_ACCESS_TOKEN= CODEX_HOME=\"~/.codex-work\" codex",
        "第三方实例启动时清空 Codex 当作官方凭据读取的变量，不用 -p"
    );
    assert_eq!(el["state"], "managed-ok", "{el}");
    assert!(
        !el.to_string().contains("codex-sentinel-value"),
        "状态输出不含取值"
    );
    // MCP 域之后再改 config.toml 的条目，不算实例配置被外部修改
    std::fs::write(
        dir.join("config.toml"),
        format!("{text}\n[mcp_servers.git]\ncommand = \"uvx\"\n"),
    )
    .unwrap();
    assert_eq!(element(&status(&ctx), "cx-work")["state"], "managed-ok");
    let db = std::fs::read(ctx.data_dir.join("native-manager.db")).unwrap();
    assert!(
        !String::from_utf8_lossy(&db).contains("codex-sentinel-value"),
        "库里零明文"
    );
    assert!(!ctx.home_dir.join(".codex").exists(), "主配置不碰");
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
            &request(
                "profiles:apply",
                &["--only", &format!("profiles:{id}"), "--base", "missing"],
            ),
        );
        assert!(!apply.ok, "[{dir}] 必须 fail-closed");
        assert_eq!(apply.errors[0].code, "PROFILE_ERROR");
    }
    // 主实例目录没有被创建或写入
    assert!(!ctx.home_dir.join(".claude/settings.json").exists());
    let _ = std::fs::remove_dir_all(root);
}

/// 实际文件为准：已有用户内容按 base 合并写入（用户自己的 env 变量保留）；外部改了实例
/// 负责的字段后状态逐字段报出差异，旧 base 写入被拒且文件不变；「采纳实际配置」把实际值
/// 写回定义（只改库），凭据不以明文进库。
#[test]
fn live_settings_are_merged_and_external_edits_can_be_adopted() {
    let (root, ctx) = temp_ctx("live-first");
    std::env::set_var("AM_TEST_TOKEN_LIVE", "live-token-value");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_TOKEN_LIVE")
    ));
    std::fs::create_dir_all(ctx.home_dir.join(".claude-ly")).unwrap();
    let target = ctx.home_dir.join(".claude-ly/settings.json");
    std::fs::write(
        &target,
        "{\"env\":{\"MINE\":\"keep\"},\"theme\":\"dark\"}\n",
    )
    .unwrap();

    let before = status(&ctx);
    let el = element(&before, "cc-ly");
    assert_eq!(el["state"], "drifted", "实际文件还没有实例定义的变量");
    assert!(el["observed"]["base"].as_str().unwrap().len() == 64);
    let apply = apply_now(&ctx, "cc-ly");
    assert!(apply.ok, "{:?}", apply.errors);
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(written["env"]["MINE"], "keep", "用户自己的 env 变量保留");
    assert_eq!(written["theme"], "dark");
    assert_eq!(
        written["env"]["ANTHROPIC_BASE_URL"],
        "https://api.example.com"
    );
    assert_eq!(element(&status(&ctx), "cc-ly")["state"], "managed-ok");

    // 外部改了实例负责的字段：状态逐字段报出，不回显令牌
    let stale = base_of(&ctx, "cc-ly");
    let mut edited = written.clone();
    edited["env"]["ANTHROPIC_BASE_URL"] = serde_json::json!("http://10.0.0.8:8080");
    edited["env"]["ANTHROPIC_AUTH_TOKEN"] = serde_json::json!("sk-edited-outside-123456");
    std::fs::write(&target, serde_json::to_string_pretty(&edited).unwrap()).unwrap();
    let now = status(&ctx);
    let el = element(&now, "cc-ly");
    assert_eq!(el["state"], "drifted");
    let fields = el["observed"]["fields"].as_array().unwrap();
    let url = fields
        .iter()
        .find(|f| f["field"] == "env.ANTHROPIC_BASE_URL")
        .unwrap();
    assert_eq!(url["state"], "differs");
    assert_eq!(url["live"], "http://10.0.0.8:8080");
    assert!(!serde_json::to_string(&now.data)
        .unwrap()
        .contains("sk-edited-outside"));
    assert!(el["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["id"] == "adopt"));

    // 旧 base 写入被拒，文件字节不变
    let bytes = std::fs::read(&target).unwrap();
    let denied = execute(
        &ctx,
        &request(
            "profiles:apply",
            &["--only", "profiles:cc-ly", "--base", &stale],
        ),
    );
    assert!(!denied.ok);
    assert!(
        denied.errors[0].message.contains("已在外部修改"),
        "{:?}",
        denied.errors
    );
    assert_eq!(std::fs::read(&target).unwrap(), bytes);

    // 采纳实际配置：地址写回定义，令牌只保留引用；文件不动
    let fresh = base_of(&ctx, "cc-ly");
    let adopt = execute(
        &ctx,
        &request(
            "profiles:adopt",
            &["--only", "profiles:cc-ly", "--base", &fresh],
        ),
    );
    assert!(adopt.ok, "{:?}", adopt.errors);
    assert!(adopt.data["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n.as_str().unwrap().contains("ANTHROPIC_AUTH_TOKEN")));
    assert_eq!(std::fs::read(&target).unwrap(), bytes, "采纳只改库");
    let policy = execute(&ctx, &request("policy:get", &[]));
    let profile = &policy.data["effective"]["agents"]["definitions"]["cc-ly"]["profile"];
    assert_eq!(profile["env"]["ANTHROPIC_BASE_URL"], "http://10.0.0.8:8080");
    assert_eq!(
        profile["env"]["ANTHROPIC_AUTH_TOKEN"],
        "${AM_TEST_TOKEN_LIVE}"
    );
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let rows: Vec<String> = conn
        .prepare("SELECT value_json FROM policy_entries")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(rows.iter().all(|row| !row.contains("sk-edited-outside")));

    // 用旧 base 采纳同样被拒
    let denied = execute(
        &ctx,
        &request(
            "profiles:adopt",
            &["--only", "profiles:cc-ly", "--base", &stale],
        ),
    );
    assert!(!denied.ok);

    std::env::remove_var("AM_TEST_TOKEN_LIVE");
    let _ = std::fs::remove_dir_all(root);
}

/// 实例 settings.json 是符号链接（dotfiles）时写到最终目标文件，链接本身不变；
/// agents:status 不再把主目录 ~/.claude 的配置当成实例的当前配置。
#[cfg(unix)]
#[test]
fn symlinked_instance_settings_are_written_through_and_agents_status_does_not_read_main_dir() {
    let (root, ctx) = temp_ctx("symlink");
    std::env::set_var("AM_TEST_TOKEN_LINK", "t");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_TOKEN_LINK")
    ));
    std::fs::create_dir_all(ctx.home_dir.join(".claude-ly")).unwrap();
    let real = root.join("dotfiles-settings.json");
    std::fs::write(&real, "{\"theme\":\"dark\"}\n").unwrap();
    let link = ctx.home_dir.join(".claude-ly/settings.json");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let apply = apply_now(&ctx, "cc-ly");
    assert!(apply.ok, "{:?}", apply.errors);
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    let text = std::fs::read_to_string(&real).unwrap();
    assert!(
        text.contains("ANTHROPIC_BASE_URL") && text.contains("\"theme\": \"dark\""),
        "{text}"
    );

    // 主目录放一份不同的配置：agents:status 对实例不得读它
    std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
    std::fs::write(
        ctx.home_dir.join(".claude/settings.json"),
        "{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://main.example.com\"}}\n",
    )
    .unwrap();
    let agents = execute(&ctx, &request("agents:status", &[]));
    assert!(agents.ok, "{:?}", agents.errors);
    let instance = agents.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "agents:cc-ly")
        .expect("实例也是一个助手定义");
    assert!(
        instance["observed"].get("current_config").is_none(),
        "实例的当前配置由 profiles:status 报告，不能读主目录"
    );
    std::env::remove_var("AM_TEST_TOKEN_LINK");
    let _ = std::fs::remove_dir_all(root);
}

/// 旧库：profile_bindings 没有 owned_json 列时读路径照常（零写），写入时补列。
#[test]
fn legacy_bindings_table_without_owned_column_still_works() {
    let (root, ctx) = temp_ctx("legacy-bindings");
    std::env::set_var("AM_TEST_TOKEN_LEGACY", "t");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_TOKEN_LEGACY")
    ));
    {
        let conn = store::open_write(&ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE profile_bindings (
                profile_id TEXT NOT NULL,
                file TEXT NOT NULL,
                applied_digest TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (profile_id, file)
            );",
        )
        .unwrap();
    }
    let db = store::db_path(&ctx);
    let before = std::fs::read(&db).unwrap();
    let _ = status(&ctx);
    assert_eq!(std::fs::read(&db).unwrap(), before, "status 零写");
    let apply = apply_now(&ctx, "cc-ly");
    assert!(apply.ok, "{:?}", apply.errors);
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let owned: String = conn
        .query_row(
            "SELECT owned_json FROM profile_bindings WHERE profile_id = 'cc-ly'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(owned.contains("env.ANTHROPIC_BASE_URL"), "{owned}");
    std::env::remove_var("AM_TEST_TOKEN_LEGACY");
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
        &request(
            "profiles:apply",
            &["--only", "profiles:cc-miss", "--base", "missing"],
        ),
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

/// 实例目录下由本工具写的是配置（env/model）；规则文件与 hooks 以实际文件为准，分别归
/// 规则页与 Hooks 页管理，profiles:apply 既不生成也不检测它们。主实例的固定路径一个字节都不动。
#[test]
fn profile_directory_leaves_rules_file_and_hooks_alone() {
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

    // 已有用户内容：按读到的实际内容（base）合并写入，不再需要接管
    let apply = apply_now(&ctx, "cc-ly");
    assert!(apply.ok, "{:?}", apply.errors);

    // 1) settings.json：本工具管的字段写入，用户字段保留
    let settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(ctx.home_dir.join(".claude-ly/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(settings["statusLine"], "mine", "用户自己的设置不得被抹掉");
    assert!(settings["env"]["ANTHROPIC_BASE_URL"].is_string());
    assert!(
        settings.get("hooks").is_none(),
        "profiles:apply 不再写 hooks 键（归 Hooks 页）"
    );

    // 2) 新建实例不自动写规则文件（用户在规则页从主助手复制或自己写）
    let instance_rules = ctx.home_dir.join(".claude-ly/CLAUDE.md");
    assert!(!instance_rules.exists(), "profiles:apply 不再生成规则文件");

    // 3) 主实例的固定路径全程未被触碰
    assert_eq!(std::fs::read(&global_settings).unwrap(), settings_before);
    assert_eq!(std::fs::read(&global_rules).unwrap(), rules_before);

    // 4) 用户自己写的实例规则与 hooks：状态不因它们变化，再次应用也不碰它们
    std::fs::write(&instance_rules, "external edit\n").unwrap();
    let mut with_hooks = settings.clone();
    with_hooks["hooks"] =
        serde_json::json!({"Stop": [{"hooks": [{"type": "command", "command": "echo mine"}]}]});
    // 与 Hooks 页写盘同一格式（pretty + 换行）
    std::fs::write(
        ctx.home_dir.join(".claude-ly/settings.json"),
        format!("{}\n", serde_json::to_string_pretty(&with_hooks).unwrap()),
    )
    .unwrap();
    let status = execute(&ctx, &request("profiles:status", &[]));
    assert_eq!(element(&status, "cc-ly")["state"], "managed-ok");
    let again = apply_now(&ctx, "cc-ly");
    assert!(again.ok, "{:?}", again.errors);
    assert_eq!(
        std::fs::read_to_string(&instance_rules).unwrap(),
        "external edit\n",
        "实例规则文件字节不变"
    );
    let after: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(ctx.home_dir.join(".claude-ly/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(after["hooks"], with_hooks["hooks"], "实例 hooks 原样保留");

    std::env::remove_var("AM_TEST_TOKEN_FULLSET");
    let _ = std::fs::remove_dir_all(root);
}

/// Codex 实例同样不生成规则文件与 hooks.json。
#[test]
fn codex_profile_directory_gets_no_rules_or_hooks_file() {
    let (root, ctx) = temp_ctx("codex-full-set");
    std::env::set_var("AM_TEST_CODEX_FULL_SET", "codex-full-set-token");
    assert!(save_definition(
        &ctx,
        "cx-work",
        r#"{"display_name":"Codex · work","bin":"codex","profile":{"base_agent":"codex","config_dir":"~/.codex-work","base_url":"https://api.example.com/v1","env_key":"AM_TEST_CODEX_FULL_SET","model":"gpt-5.5"},"_delete":false,"_replace":true}"#
    ));
    let apply = apply_now(&ctx, "cx-work");
    assert!(apply.ok, "{:?}", apply.errors);
    std::env::remove_var("AM_TEST_CODEX_FULL_SET");

    let dir = ctx.home_dir.join(".codex-work");
    assert!(dir.join("config.toml").is_file());
    assert!(!dir.join("AGENTS.md").exists(), "规则文件不由实例域生成");
    assert!(
        !dir.join("hooks.json").exists(),
        "hooks.json 以实际文件为准，归 Hooks 页"
    );

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

fn claude_settings(ctx: &Context, dir: &str) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(ctx.home_dir.join(dir).join("settings.json")).unwrap())
        .unwrap()
}

#[test]
fn 第三方实例把未声明的凭据置空_只剩自己声明的那一个() {
    let (root, ctx) = temp_ctx("cancel-env");
    std::env::set_var("AM_TEST_TOKEN_CANCEL", "cancel-token");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_TOKEN_CANCEL")
    ));
    let apply = apply_now(&ctx, "cc-ly");
    assert!(apply.ok, "{:?}", apply.errors);
    let env = &claude_settings(&ctx, ".claude-ly")["env"];
    assert_eq!(env["ANTHROPIC_AUTH_TOKEN"], "cancel-token");
    assert_eq!(
        env["ANTHROPIC_API_KEY"], "",
        "没声明的第三方凭据置空，shell 里的同名变量进不来"
    );
    assert_eq!(env["CLAUDE_CODE_OAUTH_TOKEN"], "", "官方长期令牌置空");
    assert_eq!(element(&status(&ctx), "cc-ly")["state"], "managed-ok");
    assert_eq!(
        probe_log(&ctx),
        "",
        "目录原本不存在，不需要探测，也不调用官方命令"
    );

    // 有人删掉置空项：状态报不一致，再写一次补回
    let path = ctx.home_dir.join(".claude-ly/settings.json");
    let mut live = claude_settings(&ctx, ".claude-ly");
    live["env"]
        .as_object_mut()
        .unwrap()
        .remove("CLAUDE_CODE_OAUTH_TOKEN");
    std::fs::write(&path, serde_json::to_vec_pretty(&live).unwrap()).unwrap();
    assert_eq!(element(&status(&ctx), "cc-ly")["state"], "drifted");
    let again = apply_now(&ctx, "cc-ly");
    assert!(again.ok, "{:?}", again.errors);
    assert_eq!(
        claude_settings(&ctx, ".claude-ly")["env"]["CLAUDE_CODE_OAUTH_TOKEN"],
        ""
    );
    let log = probe_log(&ctx);
    let dir = ctx.home_dir.join(".claude-ly");
    assert!(
        log.starts_with(&format!("claude|{}|", dir.display())),
        "目录已存在时按该目录探测：{log}"
    );
    std::env::remove_var("AM_TEST_TOKEN_CANCEL");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn 目录里有官方登录时拒绝写入第三方凭据_文件不变() {
    let (root, ctx) = temp_ctx("official-present");
    std::env::set_var("AM_TEST_TOKEN_OFFICIAL", "official-token");
    let dir = ctx.home_dir.join(".claude-ly");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(".fake-logged-in"), "").unwrap();
    std::fs::write(dir.join("settings.json"), "{\"theme\":\"dark\"}\n").unwrap();
    // 官方长期令牌从进程环境继承时，探测不能把它带给官方命令
    std::env::set_var("CLAUDE_CODE_OAUTH_TOKEN", "inherited-official-token");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_TOKEN_OFFICIAL")
    ));
    let refused = apply_now(&ctx, "cc-ly");
    std::env::remove_var("CLAUDE_CODE_OAUTH_TOKEN");
    assert!(!refused.ok);
    let message = format!("{:?}", refused.errors);
    assert!(message.contains("官方账号登录"), "{message}");
    assert!(!message.contains("official-token"), "错误不回显取值");
    assert_eq!(
        std::fs::read_to_string(dir.join("settings.json")).unwrap(),
        "{\"theme\":\"dark\"}\n"
    );
    assert_eq!(
        probe_log(&ctx),
        format!("claude|{}|unset\n", dir.display()),
        "探测指向实例目录，且不带进程环境里的凭据"
    );
    std::env::remove_var("AM_TEST_TOKEN_OFFICIAL");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn 第三方凭据遮住官方登录时按登录记录拒绝() {
    let (root, ctx) = temp_ctx("official-masked");
    std::env::set_var("AM_TEST_TOKEN_MASKED", "masked-token");
    let dir = ctx.home_dir.join(".claude-ly");
    std::fs::create_dir_all(&dir).unwrap();
    // 目录里已经有一份第三方凭据，官方命令只会报告它；登录会话记录在 .claude.json
    std::fs::write(
        dir.join("settings.json"),
        "{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://api.example.com\",\"ANTHROPIC_AUTH_TOKEN\":\"masked-token\"}}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join(".claude.json"),
        "{\"oauthAccount\":{\"accountUuid\":\"x\"}}\n",
    )
    .unwrap();
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_TOKEN_MASKED")
    ));
    let refused = apply_now(&ctx, "cc-ly");
    assert!(!refused.ok);
    assert!(format!("{:?}", refused.errors).contains("官方账号登录"));
    std::env::remove_var("AM_TEST_TOKEN_MASKED");
    let _ = std::fs::remove_dir_all(root);
}

/// Console 登录（claude.ai 之外的另一种官方账号登录）同样算：官方命令在目录声明的凭据被
/// 取消后报告 `apiKeySource: "/login managed key"`，Linux 上 `.claude.json` 另有 `primaryApiKey`。
#[test]
fn console_登录也算官方账号登录_拒绝写入第三方凭据() {
    let (root, ctx) = temp_ctx("official-console");
    std::env::set_var("AM_TEST_TOKEN_CONSOLE", "console-token");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_TOKEN_CONSOLE")
    ));
    let dir = ctx.home_dir.join(".claude-ly");
    std::fs::create_dir_all(&dir).unwrap();

    // 官方命令报告的 Console 登录（macOS 上 key 在钥匙串里，目录里没有文件可看）
    std::fs::write(dir.join(".fake-console"), "").unwrap();
    let refused = apply_now(&ctx, "cc-ly");
    assert!(!refused.ok);
    assert!(format!("{:?}", refused.errors).contains("官方账号登录"));
    let auth = auth_status(&ctx, "profiles:cc-ly");
    assert_eq!(auth.data["method"], "console");
    assert_eq!(auth.data["mixed"], true);
    std::fs::remove_file(dir.join(".fake-console")).unwrap();

    // Linux 上 Console 登录的 key 记在 .claude.json：只看键在不在，不读取值
    std::fs::write(
        dir.join(".claude.json"),
        "{\"primaryApiKey\":\"sk-ant-api03-not-read\"}\n",
    )
    .unwrap();
    let refused = apply_now(&ctx, "cc-ly");
    assert!(!refused.ok);
    assert!(!format!("{:?}", refused.errors).contains("sk-ant-api03"));
    assert!(!dir.join("settings.json").exists());
    std::env::remove_var("AM_TEST_TOKEN_CONSOLE");
    let _ = std::fs::remove_dir_all(root);
}

/// 探测只看这个目录存的登录：经官方 `--settings` 取消目录声明的凭据，Anthropic 配置档案目录
/// 指到不存在的位置，进程环境里的档案与联合身份变量不带给官方命令。托管设置给的 apiKeyHelper、
/// 只存 MCP 登录的 `.credentials.json` 都不是官方账号登录。
#[test]
fn 探测取消目录声明的凭据_不看共享档案_不把托管凭据当成官方登录() {
    let (root, ctx) = temp_ctx("probe-isolation");
    std::env::set_var("AM_TEST_TOKEN_PROBE", "probe-token");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_TOKEN_PROBE")
    ));
    let dir = ctx.home_dir.join(".claude-ly");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(".fake-helper"), "").unwrap();
    std::fs::write(dir.join(".credentials.json"), "{\"mcpOAuth\":{}}\n").unwrap();
    std::env::set_var("ANTHROPIC_PROFILE", "shared-wif");
    let applied = apply_now(&ctx, "cc-ly");
    std::env::remove_var("ANTHROPIC_PROFILE");
    assert!(applied.ok, "{:?}", applied.errors);

    let args = std::fs::read_to_string(ctx.home_dir.join(".probe-args")).unwrap();
    let line = args.lines().next().unwrap();
    let mut parts = line.rsplitn(3, '|');
    let profile = parts.next().unwrap();
    let config_dir = parts.next().unwrap();
    let settings: serde_json::Value = serde_json::from_str(parts.next().unwrap()).unwrap();
    assert_eq!(profile, "unset", "进程环境里的档案名不带给官方命令");
    assert!(
        config_dir.starts_with("/nonexistent/"),
        "档案目录指到不存在的位置：{config_dir}"
    );
    assert!(!std::path::Path::new(config_dir).exists());
    assert_eq!(settings["apiKeyHelper"], "");
    for name in [
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_API_KEY",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_MANTLE",
    ] {
        assert_eq!(settings["env"][name], "", "{name}");
    }
    std::env::remove_var("AM_TEST_TOKEN_PROBE");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn 无法确认有没有官方登录时拒绝写入() {
    let (root, ctx) = temp_ctx("official-unknown");
    std::env::set_var("AM_TEST_TOKEN_UNKNOWN", "unknown-token");
    std::fs::create_dir_all(ctx.home_dir.join(".claude-ly")).unwrap();
    std::fs::write(ctx.home_dir.join(".fake-garbage"), "").unwrap();
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_TOKEN_UNKNOWN")
    ));
    let refused = apply_now(&ctx, "cc-ly");
    assert!(!refused.ok);
    assert!(format!("{:?}", refused.errors).contains("无法确认"));
    assert!(!ctx.home_dir.join(".claude-ly/settings.json").exists());
    std::env::remove_var("AM_TEST_TOKEN_UNKNOWN");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn 官方账号实例不检查登录_只把外部凭据变量置空() {
    let (root, ctx) = temp_ctx("official-instance");
    let dir = ctx.home_dir.join(".claude-work");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(".fake-logged-in"), "").unwrap();
    assert!(save_definition(
        &ctx,
        "cc-work",
        r#"{"display_name":"Claude Code · work","bin":"claude","profile":{"base_agent":"claude-code","config_dir":"~/.claude-work","model":"opus"},"_delete":false,"_replace":true}"#
    ));
    let apply = apply_now(&ctx, "cc-work");
    assert!(apply.ok, "{:?}", apply.errors);
    let settings = claude_settings(&ctx, ".claude-work");
    assert_eq!(settings["model"], "opus");
    // 官方账号实例只用目录里的登录：shell 里给别的账号或中转导出的凭据、地址、云厂商开关与
    // 附加请求头一律置空取消继承，不写任何凭据取值
    let env = settings["env"].as_object().expect("写了取消继承的 env");
    for name in [
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_CUSTOM_HEADERS",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        "CLAUDE_CODE_USE_ANTHROPIC_AWS",
        "CLAUDE_CODE_USE_MANTLE",
    ] {
        assert_eq!(env[name], "", "{name}");
    }
    assert_eq!(env.len(), 10, "{env:?}");
    // 官方账号实例不跑登录检查
    assert_eq!(probe_log(&ctx), "");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn codex_第三方实例_目录已登录官方账号时拒绝_未登录才写() {
    let (root, ctx) = temp_ctx("codex-official");
    std::env::set_var("AM_TEST_CODEX_OFFICIAL", "codex-official-token");
    let dir = ctx.home_dir.join(".codex-work");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(".fake-logged-in"), "").unwrap();
    assert!(save_definition(
        &ctx,
        "cx-work",
        r#"{"display_name":"Codex · work","bin":"codex","profile":{"base_agent":"codex","config_dir":"~/.codex-work","base_url":"https://api.example.com/v1","env_key":"AM_TEST_CODEX_OFFICIAL"},"_delete":false,"_replace":true}"#
    ));
    let refused = apply_now(&ctx, "cx-work");
    assert!(!refused.ok);
    assert!(format!("{:?}", refused.errors).contains("官方账号登录"));
    assert!(!dir.join("config.toml").exists());

    std::fs::remove_file(dir.join(".fake-logged-in")).unwrap();
    let apply = apply_now(&ctx, "cx-work");
    assert!(apply.ok, "{:?}", apply.errors);
    assert!(dir.join("config.toml").exists());
    let log = probe_log(&ctx);
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        vec![
            format!("codex|{}|unset", dir.display()),
            format!("codex|{}|unset", dir.display())
        ],
        "按实例目录探测，且不带进程环境里的 CODEX_API_KEY"
    );
    std::env::remove_var("AM_TEST_CODEX_OFFICIAL");
    let _ = std::fs::remove_dir_all(root);
}

fn auth_status(ctx: &Context, only: &str) -> agent_manager_engine::Envelope {
    execute(ctx, &request("agents:auth-status", &["--only", only]))
}

#[test]
fn 登录状态用官方命令按需查询_第三方实例里有官方登录标为混用() {
    let (root, ctx) = temp_ctx("auth-status-mixed");
    std::env::set_var("AM_TEST_TOKEN_STATUS", "status-token");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_TOKEN_STATUS")
    ));
    let dir = ctx.home_dir.join(".claude-ly");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(".fake-logged-in"), "").unwrap();
    let db_before = std::fs::read(store::db_path(&ctx)).ok();

    let env = auth_status(&ctx, "profiles:cc-ly");
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["state"], "logged-in");
    assert_eq!(env.data["method"], "claude.ai");
    assert_eq!(env.data["third_party"], true);
    assert_eq!(env.data["mixed"], true);
    assert!(!env.data.to_string().contains("status-token"));
    assert_eq!(
        std::fs::read(store::db_path(&ctx)).ok(),
        db_before,
        "查询不写库"
    );
    assert_eq!(probe_log(&ctx), format!("claude|{}|unset\n", dir.display()));
    std::env::remove_var("AM_TEST_TOKEN_STATUS");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn 官方账号实例未登录_结果无法识别时如实报未知() {
    let (root, ctx) = temp_ctx("auth-status-official");
    assert!(save_definition(
        &ctx,
        "cx-work",
        r#"{"display_name":"Codex · work","bin":"codex","profile":{"base_agent":"codex","config_dir":"~/.codex-work"},"_delete":false,"_replace":true}"#
    ));
    std::fs::create_dir_all(ctx.home_dir.join(".codex-work")).unwrap();
    let env = auth_status(&ctx, "profiles:cx-work");
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["state"], "logged-out");
    assert_eq!(env.data["third_party"], false);
    assert_eq!(env.data["mixed"], false);

    assert!(save_definition(
        &ctx,
        "cc-work",
        r#"{"display_name":"Claude Code · work","bin":"claude","profile":{"base_agent":"claude-code","config_dir":"~/.claude-work"},"_delete":false,"_replace":true}"#
    ));
    std::fs::create_dir_all(ctx.home_dir.join(".claude-work")).unwrap();
    std::fs::write(ctx.home_dir.join(".fake-garbage"), "").unwrap();
    let unknown = auth_status(&ctx, "profiles:cc-work");
    assert!(unknown.ok, "{:?}", unknown.errors);
    assert_eq!(unknown.data["state"], "unknown");
    assert!(unknown.data["reason"]
        .as_str()
        .unwrap()
        .contains("无法识别"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn 主助手的登录状态不设配置目录变量_登录记录在_home_的_claude_json() {
    let (root, ctx) = temp_ctx("auth-status-main");
    let env = auth_status(&ctx, "agents:claude-code");
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["state"], "logged-out");
    assert_eq!(
        probe_log(&ctx),
        "claude||unset\n",
        "主配置不设 CLAUDE_CONFIG_DIR"
    );

    // 官方文档：主配置的登录会话记在 ~/.claude.json
    std::fs::write(
        ctx.home_dir.join(".claude.json"),
        "{\"oauthAccount\":{\"accountUuid\":\"x\"}}\n",
    )
    .unwrap();
    let env = auth_status(&ctx, "agents:claude-code");
    assert_eq!(env.data["state"], "logged-in");
    assert_eq!(env.data["mixed"], false);

    let wrong = auth_status(&ctx, "agents:nope");
    assert!(!wrong.ok);
    let _ = std::fs::remove_dir_all(root);
}
