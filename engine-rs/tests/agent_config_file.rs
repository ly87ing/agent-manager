//! 助手配置文件原文编辑：agents:config-file 只读零写并遮住凭据；agents:edit-config 按整文件
//! 基准写入，`<hidden>` 原位还原，外部修改、挪动占位符与非法内容一律拒绝且文件不变。

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

const TOKEN: &str = "tok-config-file-sentinel-7731";

#[test]
fn claude_settings_are_shown_masked_and_edited_in_place() {
    let (root, ctx) = temp_ctx("claude");
    let path = ctx.home_dir.join(".claude/settings.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "env": {"ANTHROPIC_AUTH_TOKEN": TOKEN, "MY_VAR": "1"},
            "hooks": {"Stop": [{"hooks": [{"type": "prompt", "prompt": "done?"}]}]},
            "theme": "dark"
        }))
        .unwrap(),
    )
    .unwrap();

    // 只读零写：不建库；原文里看不到令牌
    let file = read(&ctx, "agents:claude-code");
    assert!(!ctx.data_dir.exists(), "读取不得建库");
    assert_eq!(file["format"], "json");
    assert_eq!(file["hidden"], 1);
    let content = file["content"].as_str().unwrap().to_string();
    assert!(
        content.contains("<hidden>") && !content.contains(TOKEN),
        "{content}"
    );
    let base = file["base"].as_str().unwrap().to_string();

    // 用户在原文里加一条权限规则、改主题；令牌原位保留
    let mut edited: Value = serde_json::from_str(&content).unwrap();
    edited["theme"] = json!("light");
    edited["permissions"] = json!({"allow": ["Bash(npm run test *)"]});
    let text = serde_json::to_string_pretty(&edited).unwrap();
    let env = run(
        &ctx,
        "agents:edit-config",
        &[
            "--only",
            "agents:claude-code",
            "--file-base",
            &base,
            "--value",
            &text,
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["changed"], json!(["permissions", "theme"]));
    let written: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        written["env"]["ANTHROPIC_AUTH_TOKEN"], TOKEN,
        "令牌原位还原"
    );
    assert_eq!(written["theme"], "light");
    assert_eq!(written["permissions"]["allow"][0], "Bash(npm run test *)");
    assert_eq!(written["hooks"]["Stop"][0]["hooks"][0]["type"], "prompt");

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
        !detail.contains("light") && !detail.contains("npm run"),
        "{detail}"
    );
    drop(conn);

    // 旧基准：文件打开后被外部改过 → 拒绝、文件不变
    let before = std::fs::read(&path).unwrap();
    let env = run(
        &ctx,
        "agents:edit-config",
        &[
            "--only",
            "agents:claude-code",
            "--file-base",
            &base,
            "--value",
            &text,
        ],
    );
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "AGENT_CONFIG_CHANGED");
    assert!(env.errors[0].message.contains("已在外部修改"));
    assert_eq!(std::fs::read(&path).unwrap(), before);

    // 占位符挪到别处：拒绝
    let fresh = read(&ctx, "agents:claude-code");
    let moved = fresh["content"]
        .as_str()
        .unwrap()
        .replace("\"ANTHROPIC_AUTH_TOKEN\"", "\"OTHER_NAME\"");
    let env = run(
        &ctx,
        "agents:edit-config",
        &[
            "--only",
            "agents:claude-code",
            "--file-base",
            fresh["base"].as_str().unwrap(),
            "--value",
            &moved,
        ],
    );
    assert!(!env.ok);
    assert!(
        env.errors[0].message.contains("<hidden>"),
        "{:?}",
        env.errors
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);

    // 非法 JSON：拒绝
    let env = run(
        &ctx,
        "agents:edit-config",
        &[
            "--only",
            "agents:claude-code",
            "--file-base",
            fresh["base"].as_str().unwrap(),
            "--value",
            "{ not json",
        ],
    );
    assert!(!env.ok);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn codex_config_keeps_comments_and_hidden_values() {
    let (root, ctx) = temp_ctx("codex");
    let path = ctx.home_dir.join(".codex/config.toml");
    let original = format!(
        "# 我的 codex 配置\nmodel = \"o3\" # 常用\n\n[model_providers.mine]\nname = \"mine\"\nenv_key = \"MINE_KEY\"\n\n[mcp_servers.gh]\ncommand = \"gh\"\n[mcp_servers.gh.env]\nGITHUB_TOKEN = \"{TOKEN}\"\n"
    );
    std::fs::write(&path, &original).unwrap();
    let file = read(&ctx, "agents:codex");
    assert_eq!(file["format"], "toml");
    let content = file["content"].as_str().unwrap();
    assert!(
        content.contains("env_key = \"MINE_KEY\""),
        "变量名照常显示: {content}"
    );
    assert!(content.contains("GITHUB_TOKEN = \"<hidden>\"") && !content.contains(TOKEN));

    let edited = content.replace("model = \"o3\"", "model = \"o4\"");
    let env = run(
        &ctx,
        "agents:edit-config",
        &[
            "--only",
            "agents:codex",
            "--file-base",
            file["base"].as_str().unwrap(),
            "--value",
            &edited,
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["changed"], json!(["model"]));
    let written = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        written,
        original.replace("model = \"o3\"", "model = \"o4\"")
    );

    // 非法 TOML：拒绝
    let fresh = read(&ctx, "agents:codex");
    let env = run(
        &ctx,
        "agents:edit-config",
        &[
            "--only",
            "agents:codex",
            "--file-base",
            fresh["base"].as_str().unwrap(),
            "--value",
            "[broken",
        ],
    );
    assert!(!env.ok);
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
    let env = run(
        &ctx,
        "agents:edit-config",
        &[
            "--only",
            "agents:claude-code",
            "--file-base",
            file["base"].as_str().unwrap(),
            "--value",
            "{\"theme\": \"light\"}",
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(std::fs::read_to_string(&real).unwrap().contains("light"));

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
    let env = run(
        &ctx,
        "agents:edit-config",
        &[
            "--only",
            "agents:claude-code",
            "--file-base",
            file["base"].as_str().unwrap(),
            "--value",
            "{\"theme\": \"light\"}",
        ],
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

#[test]
fn length_params_stay_visible_and_pat_like_names_are_hidden() {
    let (root, ctx) = temp_ctx("classes");
    std::fs::write(
        ctx.home_dir.join(".claude/settings.json"),
        serde_json::to_vec_pretty(&json!({"env": {
            "CLAUDE_CODE_MAX_OUTPUT_TOKENS": "32000",
            "GITHUB_PAT": "plain-pat-value-1",
            "DB_PASS": "plain-pass-value-2"
        }}))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        ctx.home_dir.join(".codex/config.toml"),
        "preferred_auth_method = \"apikey\"\n",
    )
    .unwrap();
    let claude = read(&ctx, "agents:claude-code");
    let content = claude["content"].as_str().unwrap();
    assert!(content.contains("\"32000\""), "{content}");
    assert!(!content.contains("plain-pat-value-1") && !content.contains("plain-pass-value-2"));
    assert_eq!(claude["hidden"], 2);
    let codex = read(&ctx, "agents:codex");
    assert!(codex["content"].as_str().unwrap().contains("\"apikey\""));
    let _ = std::fs::remove_dir_all(root);
}
