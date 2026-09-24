//! 助手配置文件原文编辑：agents:config-file 只读零写、原样返回全文；agents:edit-config 按整文件
//! 基准写回用户给的原文，外部修改与解析不了的内容一律拒绝且文件不变；库与审计不记取值。

use agent_manager_engine::{execute, store, Context, Envelope, Request};
use serde_json::{json, Value};
use std::path::PathBuf;

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    let base = std::env::temp_dir().join(format!(
        "am-config-file-{}-{}-{:x}",
        std::process::id(),
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let ctx = Context {
        data_dir: base.join("data"),
        home_dir: base.join("home"),
    };
    std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
    std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    (base, ctx)
}

fn run(ctx: &Context, command: &str, args: &[&str]) -> Envelope {
    execute(
        ctx,
        &Request {
            command: command.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            run_id: "config-file-test".to_string(),
        },
    )
}

fn read(ctx: &Context, id: &str) -> Value {
    let env = run(ctx, "agents:config-file", &["--only", id]);
    assert!(env.ok, "{:?}", env.errors);
    env.data
}

fn edit(ctx: &Context, id: &str, base: &str, text: &str) -> Envelope {
    run(
        ctx,
        "agents:edit-config",
        &["--only", id, "--file-base", base, "--value", text],
    )
}

const TOKEN: &str = "tok-config-file-sentinel-7731";

#[test]
fn claude_settings_are_shown_and_written_back_verbatim() {
    let (root, ctx) = temp_ctx("claude");
    let path = ctx.home_dir.join(".claude/settings.json");
    // 用户自己的排版（4 空格缩进、键序）要原样保留
    let original = format!(
        "{{\n    \"env\": {{\"ANTHROPIC_AUTH_TOKEN\": \"{TOKEN}\"}},\n    \"theme\": \"dark\",\n    \"hooks\": {{\"Stop\": [{{\"hooks\": [{{\"type\": \"prompt\", \"prompt\": \"done?\"}}]}}]}}\n}}\n"
    );
    std::fs::write(&path, &original).unwrap();

    // 只读零写：不建库；全文原样返回
    let file = read(&ctx, "agents:claude-code");
    assert!(!ctx.data_dir.exists(), "读取不得建库");
    assert_eq!(file["format"], "json");
    assert_eq!(file["content"], original.as_str());
    let base = file["base"].as_str().unwrap().to_string();

    // 用户改主题、加一条权限规则：写回的就是用户给的原文
    let edited = original.replace("\"dark\"", "\"light\"").replace(
        "    \"theme\"",
        "    \"permissions\": {\"allow\": [\"Bash(npm run test *)\"]},\n    \"theme\"",
    );
    let env = edit(&ctx, "agents:claude-code", &base, &edited);
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["changed"], json!(["permissions", "theme"]));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);

    // 库与审计里没有令牌，也没有取值
    let blob: Vec<u8> = std::fs::read_dir(&ctx.data_dir)
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().is_file())
        .flat_map(|entry| std::fs::read(entry.path()).unwrap_or_default())
        .collect();
    assert!(!String::from_utf8_lossy(&blob).contains(TOKEN));
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let detail: String = conn
        .query_row(
            "SELECT detail_json FROM operation_logs WHERE operation_type = 'agents:edit-config'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        !detail.contains("light") && !detail.contains("npm run") && !detail.contains(TOKEN),
        "{detail}"
    );
    drop(conn);

    // 旧基准：文件打开后被外部改过 → 拒绝、文件不变
    let before = std::fs::read(&path).unwrap();
    let env = edit(&ctx, "agents:claude-code", &base, &original);
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "AGENT_CONFIG_CHANGED");
    assert!(env.errors[0].message.contains("已在外部修改"));
    assert_eq!(std::fs::read(&path).unwrap(), before);

    // 非法 JSON：拒绝，错误只报位置不带原文
    let fresh = read(&ctx, "agents:claude-code");
    let broken = format!("{{\n  \"env\": {{\"ANTHROPIC_AUTH_TOKEN\": \"{TOKEN}\",}}\n}}");
    let env = edit(
        &ctx,
        "agents:claude-code",
        fresh["base"].as_str().unwrap(),
        &broken,
    );
    assert!(!env.ok);
    assert!(
        env.errors[0].message.contains("第 2 行"),
        "{:?}",
        env.errors
    );
    assert!(!env.errors[0].message.contains(TOKEN));
    // 数组不是 settings.json
    let env = edit(
        &ctx,
        "agents:claude-code",
        fresh["base"].as_str().unwrap(),
        "[]",
    );
    assert!(!env.ok);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn codex_config_is_written_back_verbatim() {
    let (root, ctx) = temp_ctx("codex");
    let path = ctx.home_dir.join(".codex/config.toml");
    let original = format!(
        "# 我的 codex 配置\nmodel = \"o3\" # 常用\n\n[mcp_servers.gh]\ncommand = \"gh\"\n[mcp_servers.gh.env]\nGITHUB_TOKEN = \"{TOKEN}\"\n"
    );
    std::fs::write(&path, &original).unwrap();
    let file = read(&ctx, "agents:codex");
    assert_eq!(file["format"], "toml");
    assert_eq!(file["content"], original.as_str());

    let edited = original.replace("model = \"o3\"", "model = \"o4\"");
    let env = edit(
        &ctx,
        "agents:codex",
        file["base"].as_str().unwrap(),
        &edited,
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["changed"], json!(["model"]));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);

    // 内容没变：不写、不记审计
    let fresh = read(&ctx, "agents:codex");
    let env = edit(
        &ctx,
        "agents:codex",
        fresh["base"].as_str().unwrap(),
        &edited,
    );
    assert!(env.ok);
    assert_eq!(env.data["changed"], json!([]));

    // 非法 TOML：拒绝
    let env = edit(
        &ctx,
        "agents:codex",
        fresh["base"].as_str().unwrap(),
        "[broken",
    );
    assert!(!env.ok);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);

    // 合法 TOML 但会让 Codex 拒绝加载整份配置（真实 0.156.1 实测）：拒绝，说明原因，不回显取值
    // 顶层键要写在第一个表头之前，否则按 TOML 规则会落进前面那张表
    let fatal = format!(
        "approval_policy = \"untrusted\"\n{edited}[model_providers.x]\nbase_url = \"https://a.example.com/v1\"\n"
    );
    let env = edit(
        &ctx,
        "agents:codex",
        fresh["base"].as_str().unwrap(),
        &fatal,
    );
    assert!(!env.ok);
    let message = &env.errors[0].message;
    assert!(message.contains("拒绝加载"), "{message}");
    assert!(
        message.contains("untrusted") && message.contains("缺少 name"),
        "{message}"
    );
    assert!(!message.contains(TOKEN), "{message}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);

    // Codex 的状态同样标出这类写法
    std::fs::write(&path, &fatal).unwrap();
    let status = run(&ctx, "agents:status", &[]);
    let codex = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == "agents:codex")
        .unwrap();
    let issues = &codex["observed"]["current_config"]["config"]["fatal_issues"];
    assert_eq!(issues.as_array().unwrap().len(), 2, "{issues}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn missing_file_is_created_from_the_editor() {
    let (root, ctx) = temp_ctx("missing");
    let file = read(&ctx, "agents:claude-code");
    assert_eq!(file["exists"], false);
    assert_eq!(file["content"], "");
    assert_eq!(file["base"], "missing");
    let env = edit(
        &ctx,
        "agents:claude-code",
        "missing",
        "{\"theme\": \"dark\"}\n",
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        std::fs::read_to_string(ctx.home_dir.join(".claude/settings.json")).unwrap(),
        "{\"theme\": \"dark\"}\n"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn symlinked_settings_are_written_through_and_unknown_args_are_rejected() {
    let (root, ctx) = temp_ctx("symlink");
    let real = root.join("dotfiles-settings.json");
    std::fs::write(&real, "{\"theme\": \"dark\"}\n").unwrap();
    let link = ctx.home_dir.join(".claude/settings.json");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let file = read(&ctx, "agents:claude-code");
    let env = edit(
        &ctx,
        "agents:claude-code",
        file["base"].as_str().unwrap(),
        "{\"theme\": \"light\"}",
    );
    assert!(env.ok, "{:?}", env.errors);
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        std::fs::read_to_string(&real).unwrap(),
        "{\"theme\": \"light\"}"
    );

    let env = run(
        &ctx,
        "agents:config-file",
        &["--only", "agents:claude-code", "--bogus"],
    );
    assert!(!env.ok);
    let env = run(
        &ctx,
        "agents:edit-config",
        &["--only", "agents:claude-code", "--file-base", "missing"],
    );
    assert!(!env.ok, "缺 --value 必须拒绝");
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn io_failure_is_reported_as_is_not_as_external_change() {
    use std::os::unix::fs::PermissionsExt;
    let (root, ctx) = temp_ctx("readonly");
    let dir = ctx.home_dir.join(".claude");
    let path = dir.join("settings.json");
    std::fs::write(&path, "{\"theme\": \"dark\"}\n").unwrap();
    let file = read(&ctx, "agents:claude-code");
    // 目录只读：临时文件建不出来，写入失败但文件没被别人改过
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let env = edit(
        &ctx,
        "agents:claude-code",
        file["base"].as_str().unwrap(),
        "{\"theme\": \"light\"}",
    );
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "AGENT_ERROR", "{:?}", env.errors);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\"theme\": \"dark\"}\n"
    );
    let _ = std::fs::remove_dir_all(root);
}
