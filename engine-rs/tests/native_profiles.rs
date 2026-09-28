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

fn definition_base_of(ctx: &Context, id: &str) -> String {
    element(&status(ctx), id)["observed"]["definition_base"]
        .as_str()
        .unwrap_or_else(|| panic!("{id} 没有 definition_base"))
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

#[test]
fn saved_profile_preview_matches_write_and_rejects_changed_file_or_secret() {
    let (root, ctx) = temp_ctx("saved-profile-preview");
    std::env::set_var("AM_TEST_SAVED_PREVIEW", "preview-token");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_SAVED_PREVIEW")
    ));
    let target = ctx.home_dir.join(".claude-ly/settings.json");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let original = b"{\"theme\":\"keep\"}\n";
    std::fs::write(&target, original).unwrap();
    let global = ctx.home_dir.join(".claude/settings.json");
    std::fs::create_dir_all(global.parent().unwrap()).unwrap();
    std::fs::write(&global, b"{\"model\":\"keep\"}\n").unwrap();
    let global_before = std::fs::read(&global).unwrap();
    let base = base_of(&ctx, "cc-ly");
    let args = ["--only", "profiles:cc-ly", "--base", &base];
    let db_before = std::fs::read(ctx.data_dir.join("native-manager.db")).unwrap();
    let preview = execute(&ctx, &request("profiles:preview", &args));
    assert!(preview.ok, "{:?}", preview.errors);
    assert_eq!(std::fs::read(&target).unwrap(), original);
    assert_eq!(
        std::fs::read(ctx.data_dir.join("native-manager.db")).unwrap(),
        db_before
    );
    let candidate = preview.data["content"].as_str().unwrap();
    assert!(candidate.contains("preview-token"));
    let file_base = preview.data["file_base"].as_str().unwrap();
    let content_base = preview.data["content_base"].as_str().unwrap();
    let confirmed = [
        "--only",
        "profiles:cc-ly",
        "--base",
        &base,
        "--preview-file-base",
        file_base,
        "--preview-content-base",
        content_base,
    ];

    std::fs::write(&target, b"{\"theme\":\"changed\"}\n").unwrap();
    assert!(!execute(&ctx, &request("profiles:apply", &confirmed)).ok);
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"{\"theme\":\"changed\"}\n"
    );
    std::fs::write(&target, original).unwrap();
    std::env::set_var("AM_TEST_SAVED_PREVIEW", "new-token");
    assert!(!execute(&ctx, &request("profiles:apply", &confirmed)).ok);
    assert_eq!(std::fs::read(&target).unwrap(), original);
    std::env::set_var("AM_TEST_SAVED_PREVIEW", "preview-token");
    let done = execute(&ctx, &request("profiles:apply", &confirmed));
    assert!(done.ok, "{:?}", done.errors);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), candidate);
    assert_eq!(std::fs::read(&global).unwrap(), global_before);
    assert!(!std::fs::read(ctx.data_dir.join("native-manager.db"))
        .unwrap()
        .windows(b"preview-token".len())
        .any(|bytes| bytes == b"preview-token"));
    std::env::remove_var("AM_TEST_SAVED_PREVIEW");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn saved_codex_profile_preview_preserves_other_settings_and_matches_write() {
    let (root, ctx) = temp_ctx("saved-codex-preview");
    std::env::set_var("AM_TEST_CODEX_SAVED_PREVIEW", "codex-preview-token");
    assert!(save_definition(
        &ctx,
        "cx-work",
        r#"{"display_name":"Codex work","bin":"codex","profile":{"base_agent":"codex","kind":"codex","config_dir":"~/.codex-work","base_url":"https://relay.example.com/v1","env_key":"AM_TEST_CODEX_SAVED_PREVIEW","wire_api":"responses","model":"work-model"}}"#
    ));
    let target = ctx.home_dir.join(".codex-work/config.toml");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let original = b"[mcp_servers.fs]\ncommand = \"npx\"\n";
    std::fs::write(&target, original).unwrap();
    let base = base_of(&ctx, "cx-work");
    let preview = execute(
        &ctx,
        &request(
            "profiles:preview",
            &["--only", "profiles:cx-work", "--base", &base],
        ),
    );
    assert!(preview.ok, "{:?}", preview.errors);
    assert_eq!(std::fs::read(&target).unwrap(), original);
    let candidate = preview.data["content"].as_str().unwrap();
    let doc: toml::Table = candidate.parse().unwrap();
    assert_eq!(doc["mcp_servers"]["fs"]["command"].as_str(), Some("npx"));
    assert_eq!(doc["model"].as_str(), Some("work-model"));
    assert_eq!(
        doc["model_providers"]["agent-manager"]["experimental_bearer_token"].as_str(),
        Some("codex-preview-token")
    );
    let done = execute(
        &ctx,
        &request(
            "profiles:apply",
            &[
                "--only",
                "profiles:cx-work",
                "--base",
                &base,
                "--preview-file-base",
                preview.data["file_base"].as_str().unwrap(),
                "--preview-content-base",
                preview.data["content_base"].as_str().unwrap(),
            ],
        ),
    );
    assert!(done.ok, "{:?}", done.errors);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), candidate);
    std::env::remove_var("AM_TEST_CODEX_SAVED_PREVIEW");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn provider_candidate_updates_one_profile_definition_and_live_file_together() {
    let (root, ctx) = temp_ctx("provider-candidate");
    std::env::set_var("AM_TEST_PROVIDER_CANDIDATE", "candidate-token-value");
    let global = ctx.home_dir.join(".claude/settings.json");
    std::fs::create_dir_all(global.parent().unwrap()).unwrap();
    std::fs::write(&global, b"{\"theme\":\"keep\"}\n").unwrap();
    let global_before = std::fs::read(&global).unwrap();
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_PROVIDER_CANDIDATE")
    ));
    assert!(apply_now(&ctx, "cc-ly").ok);
    let target = ctx.home_dir.join(".claude-ly/settings.json");
    let old_file = std::fs::read(&target).unwrap();
    let base = base_of(&ctx, "cc-ly");
    let definition_base = definition_base_of(&ctx, "cc-ly");
    let candidate = r#"{"env":{"ANTHROPIC_BASE_URL":"https://new.example.com","ANTHROPIC_API_KEY":"${AM_TEST_PROVIDER_CANDIDATE}"},"model":"new-model","source_connection":"provider-demo"}"#;
    let request_update = || {
        request(
            "profiles:apply-definition",
            &[
                "--only",
                "profiles:cc-ly",
                "--base",
                &base,
                "--definition-base",
                &definition_base,
                "--candidate",
                candidate,
            ],
        )
    };
    let done = execute(&ctx, &request_update());
    assert!(done.ok, "{:?}", done.errors);
    let file: serde_json::Value = serde_json::from_slice(&std::fs::read(&target).unwrap()).unwrap();
    assert_eq!(file["env"]["ANTHROPIC_BASE_URL"], "https://new.example.com");
    assert_eq!(file["env"]["ANTHROPIC_API_KEY"], "candidate-token-value");
    assert_eq!(
        file["env"]["ANTHROPIC_AUTH_TOKEN"], "",
        "旧请求头凭据须置空以隔离 shell 继承"
    );
    assert_eq!(file["model"], "new-model");
    let policy = execute(&ctx, &request("policy:get", &[]));
    let profile = &policy.data["effective"]["agents"]["definitions"]["cc-ly"]["profile"];
    assert_eq!(profile["source_connection"], "provider-demo");
    assert_eq!(
        profile["env"]["ANTHROPIC_API_KEY"],
        "${AM_TEST_PROVIDER_CANDIDATE}"
    );
    assert_eq!(element(&status(&ctx), "cc-ly")["state"], "managed-ok");
    assert_eq!(std::fs::read(&global).unwrap(), global_before);
    assert_ne!(std::fs::read(&target).unwrap(), old_file);

    // 两个旧基准都不能重用，文件与定义保持刚刚确认过的结果。
    let finished_file = std::fs::read(&target).unwrap();
    let finished_policy = policy.data["effective"]["agents"]["definitions"]["cc-ly"].clone();
    let denied = execute(&ctx, &request_update());
    assert!(!denied.ok);
    assert_eq!(std::fs::read(&target).unwrap(), finished_file);
    let after = execute(&ctx, &request("policy:get", &[]));
    assert_eq!(
        after.data["effective"]["agents"]["definitions"]["cc-ly"],
        finished_policy
    );

    std::env::remove_var("AM_TEST_PROVIDER_CANDIDATE");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn provider_candidate_rejects_stale_file_definition_and_plaintext_before_writing() {
    let (root, ctx) = temp_ctx("provider-candidate-reject");
    std::env::set_var("AM_TEST_PROVIDER_REJECT", "candidate-token-value");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_PROVIDER_REJECT")
    ));
    assert!(apply_now(&ctx, "cc-ly").ok);
    let target = ctx.home_dir.join(".claude-ly/settings.json");
    let base = base_of(&ctx, "cc-ly");
    let definition_base = definition_base_of(&ctx, "cc-ly");
    let candidate = r#"{"env":{"ANTHROPIC_BASE_URL":"https://new.example.com","ANTHROPIC_AUTH_TOKEN":"${AM_TEST_PROVIDER_REJECT}"},"source_connection":"provider-demo"}"#;
    let update = |file_base: &str, def_base: &str, value: &str| {
        execute(
            &ctx,
            &request(
                "profiles:apply-definition",
                &[
                    "--only",
                    "profiles:cc-ly",
                    "--base",
                    file_base,
                    "--definition-base",
                    def_base,
                    "--candidate",
                    value,
                ],
            ),
        )
    };
    let policy_before = execute(&ctx, &request("policy:get", &[])).data;
    let file_before = std::fs::read(&target).unwrap();
    let bad_secret =
        r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"plaintext-token"},"source_connection":"provider-demo"}"#;
    assert!(!update(&base, &definition_base, bad_secret).ok);
    assert_eq!(std::fs::read(&target).unwrap(), file_before);
    assert_eq!(
        execute(&ctx, &request("policy:get", &[])).data,
        policy_before
    );

    std::fs::write(&target, b"{\"env\":{\"EXTERNAL\":\"keep\"}}\n").unwrap();
    let external = std::fs::read(&target).unwrap();
    assert!(!update(&base, &definition_base, candidate).ok);
    assert_eq!(std::fs::read(&target).unwrap(), external);
    assert_eq!(
        execute(&ctx, &request("policy:get", &[])).data,
        policy_before
    );

    let fresh_base = base_of(&ctx, "cc-ly");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_PROVIDER_REJECT").replace("Claude Code · ly", "Edited elsewhere")
    ));
    let updated_policy = execute(&ctx, &request("policy:get", &[])).data;
    assert!(!update(&fresh_base, &definition_base, candidate).ok);
    assert_eq!(std::fs::read(&target).unwrap(), external);
    assert_eq!(
        execute(&ctx, &request("policy:get", &[])).data,
        updated_policy
    );

    std::env::remove_var("AM_TEST_PROVIDER_REJECT");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn provider_candidate_updates_only_codex_profile_and_preserves_mcp_and_main_config() {
    let (root, ctx) = temp_ctx("provider-codex-candidate");
    std::env::set_var("AM_TEST_CODEX_OLD", "old-candidate-token");
    let global = ctx.home_dir.join(".codex/config.toml");
    std::fs::create_dir_all(global.parent().unwrap()).unwrap();
    std::fs::write(&global, b"model = \"keep\"\n").unwrap();
    let global_before = std::fs::read(&global).unwrap();
    let target = ctx.home_dir.join(".codex-work/config.toml");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"[mcp_servers.fs]\ncommand = \"npx\"\n").unwrap();
    assert!(save_definition(
        &ctx,
        "cx-work",
        r#"{"display_name":"Codex work","bin":"codex","profile":{"base_agent":"codex","kind":"codex","config_dir":"~/.codex-work","base_url":"https://old.example.com/v1","env_key":"AM_TEST_CODEX_OLD","wire_api":"responses","model":"old-model"}}"#
    ));
    assert!(apply_now(&ctx, "cx-work").ok);
    let base = base_of(&ctx, "cx-work");
    let definition_base = definition_base_of(&ctx, "cx-work");
    let candidate = r#"{"base_url":"https://new.example.com/v1","env_key":"AM_TEST_CODEX_NEW","wire_api":"responses","model":"new-model","source_connection":"provider-demo"}"#;
    let before = std::fs::read(&target).unwrap();
    let args = [
        "--only",
        "profiles:cx-work",
        "--base",
        &base,
        "--definition-base",
        &definition_base,
        "--candidate",
        candidate,
    ];
    let preview = execute(&ctx, &request("profiles:preview", &args));
    assert!(preview.ok, "{:?}", preview.errors);
    let preview_doc: toml::Table = preview.data["content"].as_str().unwrap().parse().unwrap();
    assert_eq!(preview_doc["model"].as_str(), Some("new-model"));
    assert_eq!(
        preview_doc["mcp_servers"]["fs"]["command"].as_str(),
        Some("npx")
    );
    assert_eq!(
        preview_doc["model_providers"]["agent-manager"]["experimental_bearer_token"].as_str(),
        Some("${AM_TEST_CODEX_NEW}")
    );
    assert_eq!(std::fs::read(&target).unwrap(), before);
    assert_eq!(std::fs::read(&global).unwrap(), global_before);
    std::env::set_var("AM_TEST_CODEX_NEW", "new-candidate-token");
    let done = execute(&ctx, &request("profiles:apply-definition", &args));
    assert!(done.ok, "{:?}", done.errors);
    let text = std::fs::read_to_string(&target).unwrap();
    let doc: toml::Table = text.parse().unwrap();
    assert_eq!(doc["model"].as_str(), Some("new-model"));
    assert_eq!(
        doc["model_providers"]["agent-manager"]["base_url"].as_str(),
        Some("https://new.example.com/v1")
    );
    assert_eq!(
        doc["model_providers"]["agent-manager"]["experimental_bearer_token"].as_str(),
        Some("new-candidate-token")
    );
    assert_eq!(doc["mcp_servers"]["fs"]["command"].as_str(), Some("npx"));
    assert_eq!(std::fs::read(&global).unwrap(), global_before);
    assert_eq!(element(&status(&ctx), "cx-work")["state"], "managed-ok");
    let policy = execute(&ctx, &request("policy:get", &[]));
    assert_eq!(
        policy.data["effective"]["agents"]["definitions"]["cx-work"]["profile"]["env_key"],
        "AM_TEST_CODEX_NEW"
    );
    assert!(!policy.data.to_string().contains("new-candidate-token"));

    std::env::remove_var("AM_TEST_CODEX_OLD");
    std::env::remove_var("AM_TEST_CODEX_NEW");
    let _ = std::fs::remove_dir_all(root);
}

/// 测试并行执行，环境变量是进程级共享的：每个用例用各自的变量名，互不清掉对方的。
fn claude_profile(token_var: &str) -> String {
    format!(
        r#"{{"display_name":"Claude Code · ly","bin":"claude","skills_dir":"~/.claude-ly/skills","mcp_config":{{"path":"~/.claude-ly/.claude.json","format":"json"}},"profile":{{"base_agent":"claude-code","config_dir":"~/.claude-ly","env":{{"ANTHROPIC_BASE_URL":"https://api.example.com","ANTHROPIC_AUTH_TOKEN":"${{{token_var}}}"}}}},"_delete":false,"_replace":true}}"#
    )
}

#[test]
fn draft_preview_uses_profile_renderer_without_writing_files_or_resolving_credentials() {
    let (root, ctx) = temp_ctx("draft-preview");
    let target = ctx.home_dir.join(".claude-ly/settings.json");
    let default = ctx.home_dir.join(".claude/settings.json");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::create_dir_all(default.parent().unwrap()).unwrap();
    let target_before = b"{\"theme\":\"keep\",\"env\":{\"CUSTOM\":\"untouched\"}}\n";
    std::fs::write(&target, target_before).unwrap();
    let default_before = b"{\"permissions\":{\"allow\":[\"Bash(ls:*)\"]},\"statusLine\":{\"type\":\"command\",\"command\":\"echo ready\"}}\n";
    std::fs::write(&default, default_before).unwrap();
    let draft = claude_profile("AM_PREVIEW_UNSET_TOKEN");
    let preview = execute(
        &ctx,
        &request(
            "profiles:preview",
            &["--only", "profiles:cc-ly", "--definition", &draft],
        ),
    );
    assert!(preview.ok, "{:?}", preview.errors);
    assert_eq!(preview.data["path"], "~/.claude-ly/settings.json");
    let content: serde_json::Value =
        serde_json::from_str(preview.data["content"].as_str().unwrap()).unwrap();
    assert_eq!(content["theme"], "keep");
    assert_eq!(content["env"]["CUSTOM"], "untouched");
    assert_eq!(
        content["env"]["ANTHROPIC_BASE_URL"],
        "https://api.example.com"
    );
    assert_eq!(
        content["env"]["ANTHROPIC_AUTH_TOKEN"],
        "${AM_PREVIEW_UNSET_TOKEN}"
    );
    assert_eq!(std::fs::read(&target).unwrap(), target_before);
    assert_eq!(
        std::fs::read_to_string(&default).unwrap(),
        String::from_utf8_lossy(default_before)
    );
    let seeded = execute(
        &ctx,
        &request(
            "profiles:preview",
            &[
                "--only",
                "profiles:cc-ly",
                "--definition",
                &draft,
                "--items",
                "permissions,statusline",
            ],
        ),
    );
    assert!(seeded.ok, "{:?}", seeded.errors);
    let seeded_content: serde_json::Value =
        serde_json::from_str(seeded.data["content"].as_str().unwrap()).unwrap();
    assert_eq!(seeded_content["permissions"]["allow"][0], "Bash(ls:*)");
    assert_eq!(seeded_content["statusLine"]["command"], "echo ready");
    assert_eq!(seeded.data["seed_results"][0]["state"], "copied");
    assert_eq!(seeded.data["seed_results"][1]["state"], "copied");
    assert_eq!(std::fs::read(&target).unwrap(), target_before);
    assert!(!ctx.data_dir.exists());
    assert!(probe_log(&ctx).is_empty());

    let invalid = execute(
        &ctx,
        &request(
            "profiles:preview",
            &["--only", "profiles:cc-ly", "--definition", "{}"],
        ),
    );
    assert!(!invalid.ok);
    assert_eq!(std::fs::read(&target).unwrap(), target_before);
    let primary = draft.replace("~/.claude-ly", "~/.claude");
    let blocked = execute(
        &ctx,
        &request(
            "profiles:preview",
            &["--only", "profiles:cc-ly", "--definition", &primary],
        ),
    );
    assert!(!blocked.ok);
    assert_eq!(
        std::fs::read_to_string(&default).unwrap(),
        String::from_utf8_lossy(default_before)
    );
    let new_dir = draft.replace("~/.claude-ly", "~/.claude-new");
    let missing = execute(
        &ctx,
        &request(
            "profiles:preview",
            &["--only", "profiles:cc-new", "--definition", &new_dir],
        ),
    );
    assert!(missing.ok, "{:?}", missing.errors);
    assert_eq!(missing.data["exists"], false);
    assert!(!ctx.home_dir.join(".claude-new").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn codex_draft_preview_preserves_existing_mcp_without_touching_default_config() {
    let (root, ctx) = temp_ctx("codex-draft-preview");
    let target = ctx.home_dir.join(".codex-work/config.toml");
    let default = ctx.home_dir.join(".codex/config.toml");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::create_dir_all(default.parent().unwrap()).unwrap();
    let target_before = b"[mcp_servers.fs]\ncommand = \"npx\"\n";
    std::fs::write(&target, target_before).unwrap();
    let default_before = b"model = \"keep\"\napproval_policy = \"on-request\"\n[tui]\nstatus_line = [\"model\", \"cwd\"]\n";
    std::fs::write(&default, default_before).unwrap();
    let draft = r#"{"display_name":"Codex work","bin":"codex","profile":{"base_agent":"codex","kind":"codex","config_dir":"~/.codex-work","base_url":"https://api.example.com/v1","env_key":"AM_PREVIEW_UNSET_CODEX_KEY","wire_api":"responses","model":"new-model"}}"#;
    let preview = execute(
        &ctx,
        &request(
            "profiles:preview",
            &["--only", "profiles:cx-work", "--definition", draft],
        ),
    );
    assert!(preview.ok, "{:?}", preview.errors);
    let content = preview.data["content"].as_str().unwrap();
    let doc: toml::Value = toml::from_str(content).unwrap();
    assert_eq!(doc["model"].as_str(), Some("new-model"));
    assert_eq!(doc["mcp_servers"]["fs"]["command"].as_str(), Some("npx"));
    assert_eq!(
        doc["model_providers"]["agent-manager"]["base_url"].as_str(),
        Some("https://api.example.com/v1")
    );
    assert_eq!(
        doc["model_providers"]["agent-manager"]["experimental_bearer_token"].as_str(),
        Some("${AM_PREVIEW_UNSET_CODEX_KEY}")
    );
    assert_eq!(std::fs::read(&target).unwrap(), target_before);
    assert_eq!(
        std::fs::read_to_string(&default).unwrap(),
        String::from_utf8_lossy(default_before)
    );
    let seeded = execute(
        &ctx,
        &request(
            "profiles:preview",
            &[
                "--only",
                "profiles:cx-work",
                "--definition",
                draft,
                "--items",
                "permissions,statusline",
            ],
        ),
    );
    assert!(seeded.ok, "{:?}", seeded.errors);
    let seeded_doc: toml::Value = toml::from_str(seeded.data["content"].as_str().unwrap()).unwrap();
    assert_eq!(seeded_doc["approval_policy"].as_str(), Some("on-request"));
    assert_eq!(seeded_doc["tui"]["status_line"][0].as_str(), Some("model"));
    assert_eq!(
        seeded_doc["mcp_servers"]["fs"]["command"].as_str(),
        Some("npx")
    );
    assert_eq!(seeded.data["seed_results"][0]["state"], "copied");
    assert_eq!(seeded.data["seed_results"][1]["state"], "copied");
    assert_eq!(std::fs::read(&target).unwrap(), target_before);
    assert_eq!(std::fs::read(&default).unwrap(), default_before);
    assert!(!ctx.data_dir.exists());
    assert!(probe_log(&ctx).is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn claude_seed_preview_matches_files_written_after_creation_without_writing_early() {
    let (root, ctx) = temp_ctx("claude-seed-preview");
    let source = ctx.home_dir.join(".claude");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("settings.json"), r#"{"permissions":{"allow":["Bash(ls:*)"]},"statusLine":{"type":"command","command":"echo ready"},"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo done"}]}]}}"#).unwrap();
    std::fs::write(source.join("CLAUDE.md"), "# Shared rule\n").unwrap();
    std::fs::write(
        ctx.home_dir.join(".claude.json"),
        r#"{"mcpServers":{"fs":{"type":"stdio","command":"npx","args":["fs-mcp"]},"gh":{"type":"stdio","command":"gh-mcp","env":{"GITHUB_TOKEN":"literal-token-value"}}}}"#,
    )
    .unwrap();
    let definition = r#"{"display_name":"Claude work","bin":"claude","mcp_config":{"path":"~/.claude-work/.claude.json","format":"json"},"profile":{"base_agent":"claude-code","config_dir":"~/.claude-work"}}"#;
    let preview = execute(
        &ctx,
        &request(
            "profiles:preview",
            &[
                "--only",
                "profiles:claude-work",
                "--definition",
                definition,
                "--items",
                "permissions,statusline,hooks,rules,mcp",
            ],
        ),
    );
    assert!(preview.ok, "{:?}", preview.errors);
    assert!(!ctx.home_dir.join(".claude-work").exists());
    assert!(!ctx.data_dir.exists());
    assert_eq!(preview.data["seed_results"].as_array().unwrap().len(), 5);
    assert!(preview.data["seed_results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["state"] == "copied"));
    assert_eq!(
        preview.data["seed_results"][4]["detail"]
            .as_str()
            .unwrap()
            .contains("gh"),
        true
    );
    assert!(!preview.data.to_string().contains("literal-token-value"));
    assert!(save_definition(&ctx, "claude-work", definition));
    assert!(apply_now(&ctx, "claude-work").ok);
    let seeded = seed(
        &ctx,
        "claude-work",
        "permissions,statusline,hooks,rules,mcp",
    );
    assert!(seeded.ok, "{:?}", seeded.errors);
    let dir = ctx.home_dir.join(".claude-work");
    assert_eq!(
        preview.data["content"].as_str().unwrap().as_bytes(),
        std::fs::read(dir.join("settings.json")).unwrap()
    );
    let files = preview.data["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(
        files[0]["content"].as_str().unwrap().as_bytes(),
        std::fs::read(dir.join("CLAUDE.md")).unwrap()
    );
    assert_eq!(
        files[1]["content"].as_str().unwrap().as_bytes(),
        std::fs::read(dir.join(".claude.json")).unwrap()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn codex_seed_preview_matches_files_written_after_creation_without_writing_early() {
    let (root, ctx) = temp_ctx("codex-seed-preview");
    let source = ctx.home_dir.join(".codex");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("config.toml"), "approval_policy = \"on-request\"\n[tui]\nstatus_line = [\"model\"]\n[mcp_servers.fs]\ncommand = \"npx\"\nargs = [\"fs-mcp\"]\n").unwrap();
    std::fs::write(
        source.join("hooks.json"),
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo done"}]}]}}"#,
    )
    .unwrap();
    std::fs::write(source.join("AGENTS.md"), "# Shared rule\n").unwrap();
    let definition = r#"{"display_name":"Codex work","bin":"codex","mcp_config":{"path":"~/.codex-work/config.toml","format":"toml"},"profile":{"base_agent":"codex","config_dir":"~/.codex-work","model":"never-written"}}"#;
    let unseeded = execute(
        &ctx,
        &request(
            "profiles:preview",
            &["--only", "profiles:codex-work", "--definition", definition],
        ),
    );
    assert!(unseeded.ok, "{:?}", unseeded.errors);
    assert_eq!(unseeded.data["will_write"], false);
    assert_eq!(unseeded.data["content"], "");
    let preview = execute(
        &ctx,
        &request(
            "profiles:preview",
            &[
                "--only",
                "profiles:codex-work",
                "--definition",
                definition,
                "--items",
                "permissions,statusline,hooks,rules,mcp",
            ],
        ),
    );
    assert!(preview.ok, "{:?}", preview.errors);
    assert_eq!(preview.data["will_write"], true);
    assert!(!preview.data["content"]
        .as_str()
        .unwrap()
        .contains("never-written"));
    assert!(!ctx.home_dir.join(".codex-work").exists());
    assert!(!ctx.data_dir.exists());
    assert_eq!(preview.data["seed_results"].as_array().unwrap().len(), 5);
    assert!(preview.data["seed_results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["state"] == "copied"));
    assert!(save_definition(&ctx, "codex-work", definition));
    let seeded = seed(&ctx, "codex-work", "permissions,statusline,hooks,rules,mcp");
    assert!(seeded.ok, "{:?}", seeded.errors);
    let dir = ctx.home_dir.join(".codex-work");
    assert_eq!(
        preview.data["content"].as_str().unwrap().as_bytes(),
        std::fs::read(dir.join("config.toml")).unwrap()
    );
    let files = preview.data["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(
        files[0]["content"].as_str().unwrap().as_bytes(),
        std::fs::read(dir.join("hooks.json")).unwrap()
    );
    assert_eq!(
        files[1]["content"].as_str().unwrap().as_bytes(),
        std::fs::read(dir.join("AGENTS.md")).unwrap()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn seed_preview_reports_broken_mcp_target_without_hiding_other_files_or_writing() {
    let (root, ctx) = temp_ctx("seed-preview-broken-mcp");
    let source = ctx.home_dir.join(".claude");
    let target = ctx.home_dir.join(".claude-work");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(
        source.join("settings.json"),
        r#"{"permissions":{"allow":["Bash(ls:*)"]}}"#,
    )
    .unwrap();
    std::fs::write(source.join("CLAUDE.md"), "# Shared rule\n").unwrap();
    std::fs::write(
        ctx.home_dir.join(".claude.json"),
        r#"{"mcpServers":{"fs":{"type":"stdio","command":"npx"}}}"#,
    )
    .unwrap();
    let broken = target.join(".claude.json");
    std::fs::write(&broken, b"{invalid").unwrap();
    let definition = r#"{"display_name":"Claude work","bin":"claude","mcp_config":{"path":"~/.claude-work/.claude.json","format":"json"},"profile":{"base_agent":"claude-code","config_dir":"~/.claude-work"}}"#;
    let preview = execute(
        &ctx,
        &request(
            "profiles:preview",
            &[
                "--only",
                "profiles:claude-work",
                "--definition",
                definition,
                "--items",
                "permissions,rules,mcp",
            ],
        ),
    );
    assert!(preview.ok, "{:?}", preview.errors);
    assert_eq!(preview.data["seed_results"][0]["state"], "copied");
    assert_eq!(preview.data["seed_results"][1]["state"], "copied");
    assert_eq!(preview.data["seed_results"][2]["state"], "failed");
    assert_eq!(preview.data["files"].as_array().unwrap().len(), 1);
    assert_eq!(preview.data["files"][0]["content"], "# Shared rule\n");
    assert_eq!(std::fs::read(&broken).unwrap(), b"{invalid");
    assert!(!target.join("settings.json").exists());
    assert!(!target.join("CLAUDE.md").exists());
    assert!(!ctx.data_dir.exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn provider_candidate_preview_matches_apply_without_resolving_key_or_writing() {
    let (root, ctx) = temp_ctx("provider-candidate-preview");
    std::env::set_var("AM_PREVIEW_OLD", "old-test-token");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_PREVIEW_OLD")
    ));
    assert!(apply_now(&ctx, "cc-ly").ok);
    std::env::remove_var("AM_PREVIEW_OLD");
    let target = ctx.home_dir.join(".claude-ly/settings.json");
    let before = std::fs::read(&target).unwrap();
    let policy_before = execute(&ctx, &request("policy:get", &[])).data;
    let base = base_of(&ctx, "cc-ly");
    let definition_base = definition_base_of(&ctx, "cc-ly");
    let candidate = r#"{"source_connection":"relay","env":{"ANTHROPIC_BASE_URL":"https://next.example.com","ANTHROPIC_AUTH_TOKEN":"${AM_PREVIEW_NEW}"}}"#;
    let args = [
        "--only",
        "profiles:cc-ly",
        "--base",
        &base,
        "--definition-base",
        &definition_base,
        "--candidate",
        candidate,
    ];
    let preview = execute(&ctx, &request("profiles:preview", &args));
    assert!(preview.ok, "{:?}", preview.errors);
    let preview_content: serde_json::Value =
        serde_json::from_str(preview.data["content"].as_str().unwrap()).unwrap();
    assert_eq!(
        preview_content["env"]["ANTHROPIC_BASE_URL"],
        "https://next.example.com"
    );
    assert_eq!(
        preview_content["env"]["ANTHROPIC_AUTH_TOKEN"],
        "${AM_PREVIEW_NEW}"
    );
    assert_eq!(std::fs::read(&target).unwrap(), before);
    assert_eq!(
        execute(&ctx, &request("policy:get", &[])).data,
        policy_before
    );
    assert!(probe_log(&ctx).is_empty());

    std::env::set_var("AM_PREVIEW_NEW", "new-test-token");
    let applied = execute(&ctx, &request("profiles:apply-definition", &args));
    assert!(applied.ok, "{:?}", applied.errors);
    let mut live: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&target).unwrap()).unwrap();
    assert_eq!(live["env"]["ANTHROPIC_AUTH_TOKEN"], "new-test-token");
    live["env"]["ANTHROPIC_AUTH_TOKEN"] = serde_json::json!("${AM_PREVIEW_NEW}");
    assert_eq!(live, preview_content);
    let stale = execute(&ctx, &request("profiles:preview", &args));
    assert!(!stale.ok);
    std::env::remove_var("AM_PREVIEW_NEW");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn config_file_reads_only_the_selected_profile_and_refreshes_external_edits() {
    let (root, ctx) = temp_ctx("config-file-readback");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_CONFIG_FILE")
    ));
    let target = ctx.home_dir.join(".claude-ly/settings.json");
    let default = ctx.home_dir.join(".claude/settings.json");
    std::fs::create_dir_all(default.parent().unwrap()).unwrap();
    std::fs::write(&default, "{\"default\":true}\n").unwrap();
    let args = ["--only", "profiles:cc-ly"];
    let missing = execute(&ctx, &request("profiles:config-file", &args));
    assert!(missing.ok, "{:?}", missing.errors);
    assert_eq!(missing.data["exists"], false);
    assert!(
        !target.parent().unwrap().exists(),
        "只读操作不得创建实例目录"
    );

    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(
        &target,
        "{\"model\":\"one\",\"env\":{\"TOKEN\":\"secret\"}}\n",
    )
    .unwrap();
    let first = execute(&ctx, &request("profiles:config-file", &args));
    assert_eq!(first.data["path"], "~/.claude-ly/settings.json");
    assert!(first.data["content"].as_str().unwrap().contains("\"one\""));
    std::fs::write(&target, "{\"model\":\"two\"}\n").unwrap();
    let second = execute(&ctx, &request("profiles:config-file", &args));
    assert_eq!(second.data["content"], "{\"model\":\"two\"}\n");
    let mcp = target.parent().unwrap().join(".claude.json");
    std::fs::write(&mcp, "{\"mcpServers\":{\"demo\":{}}}\n").unwrap();
    let rules = target.parent().unwrap().join("CLAUDE.md");
    std::fs::write(&rules, "# 环境规则\n").unwrap();
    for (file, path, content) in [
        (
            "mcp",
            "~/.claude-ly/.claude.json",
            "{\"mcpServers\":{\"demo\":{}}}\n",
        ),
        ("rules", "~/.claude-ly/CLAUDE.md", "# 环境规则\n"),
    ] {
        let read = execute(
            &ctx,
            &request(
                "profiles:config-file",
                &["--only", "profiles:cc-ly", "--file", file],
            ),
        );
        assert!(read.ok, "{file}: {:?}", read.errors);
        assert_eq!(read.data["path"], path);
        assert_eq!(read.data["content"], content);
    }
    assert_eq!(
        std::fs::read_to_string(&default).unwrap(),
        "{\"default\":true}\n"
    );

    assert!(save_definition(
        &ctx,
        "cx-show",
        r#"{"display_name":"Codex Show","bin":"codex","profile":{"base_agent":"codex","config_dir":"~/.codex-show","model":"gpt-demo"},"_replace":true}"#
    ));
    let codex_target = ctx.home_dir.join(".codex-show/config.toml");
    std::fs::create_dir_all(codex_target.parent().unwrap()).unwrap();
    std::fs::write(&codex_target, "model = \"gpt-demo\"\n").unwrap();
    let codex = execute(
        &ctx,
        &request("profiles:config-file", &["--only", "profiles:cx-show"]),
    );
    assert!(codex.ok, "{:?}", codex.errors);
    assert_eq!(codex.data["path"], "~/.codex-show/config.toml");
    assert_eq!(codex.data["format"], "toml");
    assert_eq!(codex.data["content"], "model = \"gpt-demo\"\n");
    let hooks = codex_target.parent().unwrap().join("hooks.json");
    std::fs::write(&hooks, "{\"hooks\":{}}\n").unwrap();
    std::fs::write(
        codex_target.parent().unwrap().join("AGENTS.md"),
        "# Codex 规则\n",
    )
    .unwrap();
    for (file, path, content) in [
        ("hooks", "~/.codex-show/hooks.json", "{\"hooks\":{}}\n"),
        ("rules", "~/.codex-show/AGENTS.md", "# Codex 规则\n"),
    ] {
        let read = execute(
            &ctx,
            &request(
                "profiles:config-file",
                &["--only", "profiles:cx-show", "--file", file],
            ),
        );
        assert!(read.ok, "{file}: {:?}", read.errors);
        assert_eq!(read.data["path"], path);
        assert_eq!(read.data["content"], content);
    }
    assert!(
        !execute(
            &ctx,
            &request(
                "profiles:config-file",
                &["--only", "profiles:cx-show", "--file", "mcp"]
            )
        )
        .ok
    );
    assert!(
        !execute(
            &ctx,
            &request(
                "profiles:config-file",
                &[
                    "--only",
                    "profiles:cc-ly",
                    "--file",
                    "../.claude/.credentials.json"
                ]
            )
        )
        .ok
    );

    let auth = ctx.home_dir.join(".claude/.credentials.json");
    std::fs::write(&auth, "dummy-auth-must-not-leak").unwrap();
    std::fs::remove_file(&target).unwrap();
    std::os::unix::fs::symlink(&auth, &target).unwrap();
    let blocked = execute(&ctx, &request("profiles:config-file", &args));
    assert!(!blocked.ok, "认证文件不能作为实例配置原文读出");
    assert!(!format!("{:?}", blocked).contains("dummy-auth-must-not-leak"));
    std::fs::remove_file(&mcp).unwrap();
    std::os::unix::fs::symlink(&auth, &mcp).unwrap();
    let blocked_mcp = execute(
        &ctx,
        &request(
            "profiles:config-file",
            &["--only", "profiles:cc-ly", "--file", "mcp"],
        ),
    );
    assert!(!blocked_mcp.ok, "认证文件不能通过 MCP 文件入口读出");
    assert!(!format!("{:?}", blocked_mcp).contains("dummy-auth-must-not-leak"));

    for bad in [
        vec!["--only", "profiles:../.claude"],
        vec!["--only", "agents:claude-code"],
        vec!["--only", "profiles:cc-ly", "--value", "x"],
    ] {
        assert!(!execute(&ctx, &request("profiles:config-file", &bad)).ok);
    }
    let _ = std::fs::remove_dir_all(root);
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
    // 带真实 id 与所属助手：界面才能列出它并提供编辑、移除
    let invalid = element(&status, "cc-plain");
    assert_eq!(invalid["state"], "unsupported");
    assert_eq!(invalid["observed"]["invalid"], true);
    assert_eq!(invalid["spec"]["base_agent"], "claude-code");
    assert_eq!(invalid["spec"]["display_name"], "cc-plain");
    assert_eq!(invalid["actions"], serde_json::json!([]));

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

    // 用户在实例文件里自己设了网关地址（定义没声明）：显示为不同、实际值可见；只剩隔离项不同
    // 时不提供「以实际配置更新定义」（采纳只搬定义声明的项，改变不了什么）
    let mut edited = settings.clone();
    edited["env"]["ANTHROPIC_BASE_URL"] = serde_json::json!("https://gw.example.com");
    std::fs::write(
        dir.join("settings.json"),
        serde_json::to_vec_pretty(&edited).unwrap(),
    )
    .unwrap();
    let env = status(&ctx);
    let el = element(&env, "cc-work");
    assert_eq!(el["state"], "drifted");
    let row = el["observed"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["field"] == "env.ANTHROPIC_BASE_URL")
        .unwrap();
    assert_eq!(row["state"], "differs");
    assert_eq!(row["live"], "https://gw.example.com");
    assert_eq!(row["purpose"], "isolation");
    let actions: Vec<&str> = el["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap())
        .collect();
    assert_eq!(actions, vec!["apply"]);
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

/// 独立环境也能测一下：按实例定义的地址、凭据变量与模型发最小请求，不写实例文件；官方账号
/// 环境不代为发请求；凭据变量没取值时不发请求。
#[test]
fn 独立环境可检查可用性_只读_官方账号环境不代为检查() {
    let (root, ctx) = temp_ctx("profile-check");
    // 端口 1 立即拒绝：要的是「按实例定义拼出了端点、没写任何东西」，不是真去连
    assert!(save_definition(
        &ctx,
        "cc-relay",
        r#"{"display_name":"relay","bin":"claude","profile":{"base_agent":"claude-code","config_dir":"~/.claude-relay","model":"glm-5.3","env":{"ANTHROPIC_BASE_URL":"https://127.0.0.1:1/api/anthropic","ANTHROPIC_API_KEY":"${AM_TEST_PROFILE_CHECK_KEY}"}},"_delete":false,"_replace":true}"#
    ));
    assert!(save_definition(
        &ctx,
        "cx-relay",
        r#"{"display_name":"cx","bin":"codex","profile":{"base_agent":"codex","config_dir":"~/.codex-relay","base_url":"https://127.0.0.1:1/v1","env_key":"AM_TEST_PROFILE_CHECK_KEY","model":"m1"},"_delete":false,"_replace":true}"#
    ));
    assert!(save_definition(
        &ctx,
        "cc-home",
        r#"{"display_name":"home","bin":"claude","profile":{"base_agent":"claude-code","config_dir":"~/.claude-home"},"_delete":false,"_replace":true}"#
    ));
    let check = |id: &str| execute(&ctx, &request("agents:check", &["--only", id]));

    let unset = check("profiles:cc-relay");
    assert!(!unset.ok);
    assert!(
        unset.errors[0]
            .message
            .contains("AM_TEST_PROFILE_CHECK_KEY"),
        "{:?}",
        unset.errors
    );

    std::env::set_var("AM_TEST_PROFILE_CHECK_KEY", "check-secret-value");
    let claude = check("profiles:cc-relay");
    assert!(claude.ok, "{:?}", claude.errors);
    assert_eq!(claude.data["element"], "profiles:cc-relay");
    assert_eq!(
        claude.data["endpoint"],
        "https://127.0.0.1:1/api/anthropic/v1/messages"
    );
    assert_eq!(claude.data["model"], "glm-5.3");
    assert_eq!(claude.data["ok"], false, "端口 1 连不上");
    let codex = check("profiles:cx-relay");
    assert!(codex.ok, "{:?}", codex.errors);
    assert_eq!(codex.data["endpoint"], "https://127.0.0.1:1/v1/responses");
    let text = format!("{:?}{:?}", claude.data, codex.data);
    assert!(!text.contains("check-secret-value"), "结果不含取值");

    let official = check("profiles:cc-home");
    assert!(!official.ok);
    assert!(
        official.errors[0].message.contains("官方账号"),
        "{:?}",
        official.errors
    );
    let missing = check("profiles:nope");
    assert!(!missing.ok);

    // 只读：实例目录一个都没建
    for dir in [".claude-relay", ".codex-relay", ".claude-home"] {
        assert!(!ctx.home_dir.join(dir).exists(), "{dir}");
    }
    std::env::remove_var("AM_TEST_PROFILE_CHECK_KEY");
    let _ = std::fs::remove_dir_all(root);
}

/// 查余额只对有官方余额接口的厂商；变量没取值时在发请求前拒绝。
#[test]
fn 查余额只对有官方接口的厂商_没取值不发请求() {
    let (root, ctx) = temp_ctx("profile-balance");
    assert!(save_definition(
        &ctx,
        "cc-glm",
        r#"{"display_name":"glm","bin":"claude","profile":{"base_agent":"claude-code","config_dir":"~/.claude-glm","env":{"ANTHROPIC_BASE_URL":"https://open.bigmodel.cn/api/anthropic","ANTHROPIC_AUTH_TOKEN":"${AM_TEST_BALANCE_GLM}"}},"_delete":false,"_replace":true}"#
    ));
    assert!(save_definition(
        &ctx,
        "cc-ds",
        r#"{"display_name":"ds","bin":"claude","profile":{"base_agent":"claude-code","config_dir":"~/.claude-ds","env":{"ANTHROPIC_BASE_URL":"https://api.deepseek.com/anthropic","ANTHROPIC_AUTH_TOKEN":"${AM_TEST_BALANCE_UNSET}"}},"_delete":false,"_replace":true}"#
    ));
    let glm = execute(
        &ctx,
        &request("agents:balance", &["--only", "profiles:cc-glm"]),
    );
    assert!(!glm.ok);
    assert!(
        glm.errors[0].message.contains("没有公开的余额查询接口"),
        "{:?}",
        glm.errors
    );
    let ds = execute(
        &ctx,
        &request("agents:balance", &["--only", "profiles:cc-ds"]),
    );
    assert!(!ds.ok);
    assert!(
        ds.errors[0].message.contains("AM_TEST_BALANCE_UNSET"),
        "{:?}",
        ds.errors
    );
    // 状态里 Claude 实例也给出地址，界面据此决定给不给「查余额」
    let status = status(&ctx);
    assert_eq!(
        element(&status, "cc-ds")["spec"]["base_url"],
        "https://api.deepseek.com/anthropic"
    );
    let _ = std::fs::remove_dir_all(root);
}

fn seed(ctx: &Context, id: &str, items: &str) -> agent_manager_engine::Envelope {
    execute(
        ctx,
        &request(
            "profiles:seed",
            &["--only", &format!("profiles:{id}"), "--items", items],
        ),
    )
}

fn shared_status(ctx: &Context, id: &str) -> agent_manager_engine::Envelope {
    execute(
        ctx,
        &request(
            "profiles:shared-status",
            &["--only", &format!("profiles:{id}")],
        ),
    )
}

fn shared_item(status: &agent_manager_engine::Envelope, name: &str) -> serde_json::Value {
    status.data["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["item"] == name)
        .unwrap_or_else(|| panic!("缺少共用设置项 {name}"))
        .clone()
}

fn apply_shared(
    ctx: &Context,
    id: &str,
    item: &serde_json::Value,
) -> agent_manager_engine::Envelope {
    execute(
        ctx,
        &request(
            "profiles:shared-apply",
            &[
                "--only",
                &format!("profiles:{id}"),
                "--item",
                item["item"].as_str().unwrap(),
                "--source-base",
                item["source_base"].as_str().unwrap(),
                "--target-base",
                item["target_base"].as_str().unwrap(),
            ],
        ),
    )
}

fn preview_shared(ctx: &Context, id: &str, items: &[&str]) -> agent_manager_engine::Envelope {
    let status = shared_status(ctx, id);
    assert!(status.ok, "{:?}", status.errors);
    let selection: Vec<_> = items
        .iter()
        .map(|name| {
            let item = shared_item(&status, name);
            serde_json::json!({
                "item": name,
                "source_base": item["source_base"],
                "target_base": item["target_base"],
            })
        })
        .collect();
    let value = serde_json::to_string(&selection).unwrap();
    execute(
        ctx,
        &request(
            "profiles:shared-preview",
            &["--only", &format!("profiles:{id}"), "--selection", &value],
        ),
    )
}

#[test]
fn 已建环境多项预览与逐项写入完全一致且预览零写() {
    let (root, ctx) = temp_ctx("shared-preview-all");
    std::env::set_var("AM_TEST_SHARED_PREVIEW", "YOUR_TEST_TOKEN");
    let main_dir = ctx.home_dir.join(".claude");
    std::fs::create_dir_all(&main_dir).unwrap();
    std::fs::write(
        main_dir.join("settings.json"),
        serde_json::to_vec(&serde_json::json!({
            "permissions": {"allow": ["Bash(ls:*)"]},
            "statusLine": {"type": "command", "command": "echo source"},
            "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "echo source"}]}]},
            "env": {"ANTHROPIC_AUTH_TOKEN": "YOUR_MAIN_TEST_TOKEN"}
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(main_dir.join("CLAUDE.md"), "# 来源规则\n").unwrap();
    std::fs::write(ctx.home_dir.join(".claude.json"), r#"{"mcpServers":{"fs":{"type":"stdio","command":"source-fs"},"private":{"type":"stdio","command":"private-tool","env":{"API_KEY":"YOUR_LITERAL_TEST_KEY"}}}}"#).unwrap();
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_SHARED_PREVIEW")
    ));
    assert!(apply_now(&ctx, "cc-ly").ok);
    let dir = ctx.home_dir.join(".claude-ly");
    let main = dir.join("settings.json");
    let mut doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&main).unwrap()).unwrap();
    doc["permissions"] = serde_json::json!({"allow": ["Bash(pwd:*)"]});
    doc["statusLine"] = serde_json::json!({"type": "command", "command": "echo own"});
    doc["hooks"] =
        serde_json::json!({"Stop": [{"hooks": [{"type": "command", "command": "echo own"}]}]});
    std::fs::write(&main, serde_json::to_vec(&doc).unwrap()).unwrap();
    std::fs::write(dir.join("CLAUDE.md"), "# 环境自己的规则\n").unwrap();
    std::fs::write(
        dir.join(".claude.json"),
        r#"{"mcpServers":{"fs":{"type":"stdio","command":"own-fs"}}}"#,
    )
    .unwrap();
    let items = ["permissions", "statusline", "hooks", "rules", "mcp:fs"];
    let before = snapshot_dir(&dir);
    let db_before = std::fs::read(store::db_path(&ctx)).unwrap();
    let preview = preview_shared(&ctx, "cc-ly", &items);
    assert!(preview.ok, "{:?}", preview.errors);
    assert!(
        preview.data["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["state"] == "ready"),
        "{}",
        preview.data
    );
    assert_eq!(std::fs::read(store::db_path(&ctx)).unwrap(), db_before);
    assert_eq!(snapshot_dir(&dir), before);
    let files = preview.data["files"].as_array().unwrap();
    assert_eq!(files.len(), 3, "{}", preview.data);
    let status = shared_status(&ctx, "cc-ly");
    let old_line = shared_item(&status, "statusline");
    let selection = serde_json::json!([{
        "item": "statusline",
        "source_base": old_line["source_base"],
        "target_base": old_line["target_base"],
    }])
    .to_string();
    let source_path = main_dir.join("settings.json");
    let original_source = std::fs::read(&source_path).unwrap();
    let mut changed_source: serde_json::Value = serde_json::from_slice(&original_source).unwrap();
    changed_source["statusLine"]["command"] = serde_json::json!("echo changed outside");
    std::fs::write(&source_path, serde_json::to_vec(&changed_source).unwrap()).unwrap();
    let stale = execute(
        &ctx,
        &request(
            "profiles:shared-preview",
            &["--only", "profiles:cc-ly", "--selection", &selection],
        ),
    );
    assert!(!stale.ok);
    assert!(stale.errors[0].message.contains("已变化"));
    assert_eq!(snapshot_dir(&dir), before);
    assert_eq!(std::fs::read(store::db_path(&ctx)).unwrap(), db_before);
    std::fs::write(&source_path, original_source).unwrap();
    for name in items {
        let result = apply_shared(&ctx, "cc-ly", &shared_item(&status, name));
        assert!(result.ok, "{name}: {:?}", result.errors);
    }
    for file in files {
        assert_eq!(
            std::fs::read(file["path"].as_str().unwrap()).unwrap(),
            file["content"].as_str().unwrap().as_bytes()
        );
    }
    let blocked = preview_shared(&ctx, "cc-ly", &["mcp:private"]);
    assert!(!blocked.ok);
    assert!(!blocked.data.to_string().contains("YOUR_LITERAL_TEST_KEY"));
    std::env::remove_var("AM_TEST_SHARED_PREVIEW");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn 已建_claude_环境逐项更新共用设置_来源目标变化拒绝覆盖() {
    let (root, ctx) = temp_ctx("shared-claude");
    std::env::set_var("AM_TEST_SHARED_CLAUDE", "YOUR_TEST_TOKEN");
    let main_dir = ctx.home_dir.join(".claude");
    std::fs::create_dir_all(&main_dir).unwrap();
    let main_settings = main_dir.join("settings.json");
    std::fs::write(
        &main_settings,
        serde_json::to_vec(&serde_json::json!({
            "permissions": {"allow": ["Bash(ls:*)"]},
            "statusLine": {"type": "command", "command": "echo source"},
            "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "echo source"}]}]},
            "env": {"ANTHROPIC_AUTH_TOKEN": "YOUR_MAIN_TEST_TOKEN"}
        }))
        .unwrap(),
    )
    .unwrap();
    let main_rules = main_dir.join("CLAUDE.md");
    std::fs::write(&main_rules, "# 来源规则\n").unwrap();
    let main_mcp = ctx.home_dir.join(".claude.json");
    std::fs::write(&main_mcp, serde_json::to_vec(&serde_json::json!({"mcpServers": {
        "fs": {"type": "stdio", "command": "source-fs"},
        "private": {"type": "stdio", "command": "private-tool", "env": {"API_KEY": "YOUR_LITERAL_TEST_KEY"}}
    }})).unwrap()).unwrap();
    let auth = main_dir.join("auth.json");
    std::fs::write(&auth, "YOUR_OFFICIAL_LOGIN_MARKER").unwrap();
    let main_before = [
        main_settings.clone(),
        main_rules.clone(),
        main_mcp.clone(),
        auth.clone(),
    ]
    .map(|path| std::fs::read(path).unwrap());
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_SHARED_CLAUDE")
    ));
    assert!(apply_now(&ctx, "cc-ly").ok);
    let dir = ctx.home_dir.join(".claude-ly");
    let target = dir.join("settings.json");
    let mut doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&target).unwrap()).unwrap();
    doc["permissions"] = serde_json::json!({"allow": ["Bash(pwd:*)"]});
    doc["statusLine"] = serde_json::json!({"type": "command", "command": "echo own"});
    doc["hooks"] =
        serde_json::json!({"Stop": [{"hooks": [{"type": "command", "command": "echo own"}]}]});
    std::fs::write(&target, serde_json::to_vec(&doc).unwrap()).unwrap();
    std::fs::write(dir.join("CLAUDE.md"), "# 环境自己的规则\n").unwrap();
    std::fs::write(
        dir.join(".claude.json"),
        r#"{"mcpServers":{"fs":{"type":"stdio","command":"own-fs"}}}"#,
    )
    .unwrap();
    let target_before = std::fs::read(&target).unwrap();
    let preview = shared_status(&ctx, "cc-ly");
    assert!(preview.ok, "{:?}", preview.errors);
    assert_eq!(std::fs::read(&target).unwrap(), target_before, "预览零写");
    for item in ["permissions", "statusline", "hooks", "rules", "mcp:fs"] {
        assert_eq!(shared_item(&preview, item)["state"], "different", "{item}");
    }
    assert_eq!(shared_item(&preview, "mcp:private")["state"], "blocked");
    assert!(!preview.data.to_string().contains("YOUR_LITERAL_TEST_KEY"));
    assert!(!preview.data.to_string().contains("YOUR_MAIN_TEST_TOKEN"));
    let status_fields = &shared_item(&preview, "statusline")["field_diffs"];
    assert!(status_fields.to_string().contains("内容隐藏"));
    assert!(!status_fields.to_string().contains("echo source"));
    assert!(!status_fields.to_string().contains("echo own"));

    let permissions = shared_item(&preview, "permissions");
    assert!(apply_shared(&ctx, "cc-ly", &permissions).ok);
    let after_permissions: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&target).unwrap()).unwrap();
    assert_eq!(after_permissions["permissions"]["allow"][0], "Bash(ls:*)");
    assert_eq!(
        after_permissions["statusLine"]["command"], "echo own",
        "未选项保持原样"
    );
    assert_eq!(after_permissions["env"], doc["env"], "实例接入凭据保持原样");
    let conn = store::open_write(&ctx).unwrap();
    let audit: String = conn
        .query_row(
            "SELECT detail_json FROM operation_logs WHERE operation_type = 'profiles:shared-apply' AND target = 'profiles:cc-ly'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(audit.contains("permissions"));
    assert!(!audit.contains("YOUR_MAIN_TEST_TOKEN"));
    conn.execute_batch(
        "CREATE TRIGGER fail_shared_audit BEFORE INSERT ON operation_logs
         WHEN NEW.operation_type = 'profiles:shared-apply'
         BEGIN SELECT RAISE(ABORT, 'injected-audit-failure'); END;",
    )
    .unwrap();
    let statusline = shared_item(&preview, "statusline");
    let before_failed_apply = std::fs::read(&target).unwrap();
    let failed = apply_shared(&ctx, "cc-ly", &statusline);
    assert!(!failed.ok);
    assert_eq!(
        std::fs::read(&target).unwrap(),
        before_failed_apply,
        "审计失败必须恢复目标"
    );
    conn.execute_batch("DROP TRIGGER fail_shared_audit;")
        .unwrap();
    drop(conn);
    assert!(
        !apply_shared(&ctx, "cc-ly", &permissions).ok,
        "旧目标基准不可重用"
    );

    let mut source: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&main_settings).unwrap()).unwrap();
    source["statusLine"]["command"] = serde_json::json!("echo later");
    std::fs::write(&main_settings, serde_json::to_vec(&source).unwrap()).unwrap();
    assert!(
        !apply_shared(&ctx, "cc-ly", &statusline).ok,
        "来源变化必须拒绝"
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&target).unwrap()).unwrap()
            ["statusLine"]["command"],
        "echo own"
    );
    let fresh = shared_status(&ctx, "cc-ly");
    for item in ["statusline", "hooks", "rules", "mcp:fs"] {
        let result = apply_shared(&ctx, "cc-ly", &shared_item(&fresh, item));
        assert!(result.ok, "{item}: {:?}", result.errors);
    }
    let done = shared_status(&ctx, "cc-ly");
    for item in ["permissions", "statusline", "hooks", "rules", "mcp:fs"] {
        assert_eq!(shared_item(&done, item)["state"], "same", "{item}");
    }
    assert_eq!(shared_item(&done, "mcp:private")["state"], "blocked");
    for (path, original) in [main_settings, main_rules, main_mcp, auth]
        .into_iter()
        .zip(main_before)
    {
        if path.ends_with("settings.json") {
            assert_eq!(
                std::fs::read(path).unwrap(),
                serde_json::to_vec(&source).unwrap()
            );
        } else {
            assert_eq!(
                std::fs::read(path).unwrap(),
                original,
                "主配置和登录文件不被写入"
            );
        }
    }
    std::env::remove_var("AM_TEST_SHARED_CLAUDE");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn 已建_codex_环境更新审批状态栏_保留接入和官方登录() {
    let (root, ctx) = temp_ctx("shared-codex");
    std::env::set_var("AM_TEST_SHARED_CODEX", "profile-codex-token");
    let main_dir = ctx.home_dir.join(".codex");
    std::fs::create_dir_all(&main_dir).unwrap();
    let main_config = main_dir.join("config.toml");
    std::fs::write(&main_config, "approval_policy = \"on-request\"\nsandbox_mode = \"workspace-write\"\n\n[tui]\nstatus_line = [\"model\", \"cwd\"]\n\n[mcp_servers.fs]\ncommand = \"source-fs\"\n").unwrap();
    std::fs::write(
        main_dir.join("hooks.json"),
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo source"}]}]}}"#,
    )
    .unwrap();
    std::fs::write(main_dir.join("AGENTS.md"), "# 来源规则\n").unwrap();
    let auth = main_dir.join("auth.json");
    std::fs::write(&auth, "official-login-marker").unwrap();
    let source_before = std::fs::read(&main_config).unwrap();
    let auth_before = std::fs::read(&auth).unwrap();
    assert!(save_definition(
        &ctx,
        "cx-work",
        r#"{"display_name":"Codex · work","bin":"codex","mcp_config":{"path":"~/.codex-work/config.toml","format":"toml"},"profile":{"base_agent":"codex","config_dir":"~/.codex-work","base_url":"https://api.example.com/v1","env_key":"AM_TEST_SHARED_CODEX","model":"gpt-5.5"},"_delete":false,"_replace":true}"#
    ));
    assert!(apply_now(&ctx, "cx-work").ok);
    let target = ctx.home_dir.join(".codex-work/config.toml");
    // 当前实例文件已有自己的接入表；另外给五类共用设置各留一份环境自己的取值。
    let mut parsed: toml_edit::Document =
        std::fs::read_to_string(&target).unwrap().parse().unwrap();
    parsed["approval_policy"] = toml_edit::value("never");
    parsed["tui"] = toml_edit::Item::Table(toml_edit::Table::new());
    let mut own_line = toml_edit::Array::new();
    own_line.push("cwd");
    parsed["tui"]["status_line"] = toml_edit::value(own_line);
    std::fs::write(
        &target,
        format!("{}\n[mcp_servers.fs]\ncommand = \"own-fs\"\n", parsed),
    )
    .unwrap();
    let dir = ctx.home_dir.join(".codex-work");
    std::fs::write(
        dir.join("hooks.json"),
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo own"}]}]}}"#,
    )
    .unwrap();
    std::fs::write(dir.join("AGENTS.md"), "# 环境自己的规则\n").unwrap();
    let before: toml::Value = std::fs::read_to_string(&target).unwrap().parse().unwrap();
    let preview = shared_status(&ctx, "cx-work");
    assert!(preview.ok, "{:?}", preview.errors);
    assert_eq!(shared_item(&preview, "permissions")["state"], "different");
    assert_eq!(shared_item(&preview, "statusline")["state"], "different");
    for item in ["hooks", "rules", "mcp:fs"] {
        assert_eq!(shared_item(&preview, item)["state"], "different", "{item}");
    }
    let permission_item = shared_item(&preview, "permissions");
    let approval = permission_item["field_diffs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["field"] == "approval_policy")
        .unwrap();
    assert_eq!(approval["source"], "on-request");
    assert_eq!(approval["target"], "never");
    let statusline_item = shared_item(&preview, "statusline");
    let status_line = statusline_item["field_diffs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["field"] == "status_line")
        .unwrap();
    assert_eq!(status_line["source"], "2 项（内容隐藏）");
    assert_eq!(status_line["target"], "1 项（内容隐藏）");
    let original_bytes = std::fs::read(&target).unwrap();
    let db_before = std::fs::read(store::db_path(&ctx)).unwrap();
    let selected = preview_shared(
        &ctx,
        "cx-work",
        &["statusline", "permissions", "hooks", "rules", "mcp:fs"],
    );
    assert!(selected.ok, "{:?}", selected.errors);
    assert!(
        selected.data["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["state"] == "ready"),
        "{}",
        selected.data
    );
    assert_eq!(selected.data["files"].as_array().unwrap().len(), 3);
    assert_eq!(std::fs::read(&target).unwrap(), original_bytes);
    assert_eq!(std::fs::read(store::db_path(&ctx)).unwrap(), db_before);
    assert!(apply_shared(&ctx, "cx-work", &shared_item(&preview, "permissions")).ok);
    assert!(apply_shared(&ctx, "cx-work", &shared_item(&preview, "statusline")).ok);
    for item in ["hooks", "rules", "mcp:fs"] {
        let result = apply_shared(&ctx, "cx-work", &shared_item(&preview, item));
        assert!(result.ok, "{item}: {:?}", result.errors);
    }
    for file in selected.data["files"].as_array().unwrap() {
        assert_eq!(
            std::fs::read(file["path"].as_str().unwrap()).unwrap(),
            file["content"].as_str().unwrap().as_bytes(),
            "{}",
            file["path"]
        );
    }
    let after: toml::Value = std::fs::read_to_string(&target).unwrap().parse().unwrap();
    assert_eq!(after["approval_policy"].as_str(), Some("on-request"));
    assert_eq!(after["sandbox_mode"].as_str(), Some("workspace-write"));
    assert_eq!(
        after["tui"]["status_line"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(after["model_provider"], before["model_provider"]);
    assert_eq!(after["model_providers"], before["model_providers"]);
    assert_eq!(std::fs::read(&main_config).unwrap(), source_before);
    assert_eq!(std::fs::read(&auth).unwrap(), auth_before);
    std::env::remove_var("AM_TEST_SHARED_CODEX");
    let _ = std::fs::remove_dir_all(root);
}

fn seed_states(env: &agent_manager_engine::Envelope) -> Vec<(String, String)> {
    env.data["results"]
        .as_array()
        .expect("有 results")
        .iter()
        .map(|r| {
            (
                r["item"].as_str().unwrap().to_string(),
                r["state"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn states(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

/// 实例目录下全部文件的字节快照（比对「没有写」用）。
fn snapshot_dir(dir: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(&path).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.clone(), std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn 默认环境的设置可一次性带进_claude_独立环境_只补没有的_不带凭据() {
    let (root, ctx) = temp_ctx("seed-claude");
    std::env::set_var("AM_TEST_SEED_CLAUDE_TOKEN", "seed-token-value");
    let main_dir = ctx.home_dir.join(".claude");
    std::fs::create_dir_all(&main_dir).unwrap();
    let main_settings = serde_json::json!({
        "permissions": {"allow": ["Bash(ls:*)"], "defaultMode": "acceptEdits"},
        "statusLine": {"type": "command", "command": "~/.claude/statusline.sh"},
        "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "echo done"}]}]},
        "env": {"ANTHROPIC_AUTH_TOKEN": "main-secret-token"},
        "apiKeyHelper": "~/bin/key.sh"
    });
    std::fs::write(
        main_dir.join("settings.json"),
        serde_json::to_vec_pretty(&main_settings).unwrap(),
    )
    .unwrap();
    std::fs::write(main_dir.join("CLAUDE.md"), "# 我的规则\n").unwrap();
    let main_claude_json = serde_json::json!({
        "oauthAccount": {"emailAddress": "YOUR_EMAIL"},
        "mcpServers": {
            "fs": {"type": "stdio", "command": "npx", "args": ["-y", "fs-mcp"], "env": {"LOG": "debug"}},
            "gh": {"type": "stdio", "command": "gh-mcp", "env": {"GITHUB_TOKEN": "literal-token-value"}},
            "dup": {"type": "stdio", "command": "from-main"}
        }
    });
    std::fs::write(
        ctx.home_dir.join(".claude.json"),
        serde_json::to_vec_pretty(&main_claude_json).unwrap(),
    )
    .unwrap();
    let main_before = snapshot_dir(&main_dir);
    let main_json_before = std::fs::read(ctx.home_dir.join(".claude.json")).unwrap();

    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_SEED_CLAUDE_TOKEN")
    ));
    let dir = ctx.home_dir.join(".claude-ly");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(".claude.json"),
        r#"{"mcpServers":{"dup":{"type":"stdio","command":"instance-own"}}}"#,
    )
    .unwrap();
    let apply = apply_now(&ctx, "cc-ly");
    assert!(apply.ok, "{:?}", apply.errors);
    let state_before = element(&status(&ctx), "cc-ly")["state"].clone();
    let before_preview = snapshot_dir(&dir);
    let preview = execute(
        &ctx,
        &request(
            "profiles:preview",
            &[
                "--only",
                "profiles:cc-ly",
                "--definition",
                &claude_profile("AM_TEST_SEED_CLAUDE_TOKEN"),
                "--items",
                "permissions,statusline,hooks,rules,mcp",
            ],
        ),
    );
    assert!(preview.ok, "{:?}", preview.errors);
    assert_eq!(snapshot_dir(&dir), before_preview, "预览不能写实例目录");

    let env = seed(&ctx, "cc-ly", "permissions,statusline,hooks,rules,mcp");
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        seed_states(&env),
        states(&[
            ("permissions", "copied"),
            ("statusline", "copied"),
            ("hooks", "copied"),
            ("rules", "copied"),
            ("mcp", "copied"),
        ])
    );
    let mcp = &env.data["results"][4];
    assert_eq!(mcp["skipped"], serde_json::json!(["gh"]), "{mcp}");
    let detail = mcp["detail"].as_str().unwrap();
    assert!(
        detail.contains("已复制 fs") && detail.contains("dup"),
        "{detail}"
    );
    assert!(!detail.contains("literal-token-value"));

    let settings = claude_settings(&ctx, ".claude-ly");
    assert_eq!(settings["permissions"], main_settings["permissions"]);
    assert_eq!(settings["statusLine"], main_settings["statusLine"]);
    assert_eq!(settings["hooks"], main_settings["hooks"]);
    assert!(settings.get("apiKeyHelper").is_none(), "凭据命令不带");
    assert_eq!(
        settings["env"]["ANTHROPIC_AUTH_TOKEN"], "seed-token-value",
        "实例自己的凭据不被主配置的 env 覆盖"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap(),
        "# 我的规则\n"
    );
    let instance_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(
        preview.data["files"][1]["content"]
            .as_str()
            .unwrap()
            .as_bytes(),
        std::fs::read(dir.join(".claude.json")).unwrap()
    );
    let servers = instance_json["mcpServers"].as_object().unwrap();
    assert_eq!(servers["fs"]["command"], "npx");
    assert_eq!(servers["dup"]["command"], "instance-own", "同名条目不动");
    assert!(!servers.contains_key("gh"), "含明文凭据的条目不带");
    assert!(
        instance_json.get("oauthAccount").is_none(),
        "不带主配置的账号字段"
    );

    // 主配置一个字节都没动
    assert_eq!(snapshot_dir(&main_dir), main_before);
    assert_eq!(
        std::fs::read(ctx.home_dir.join(".claude.json")).unwrap(),
        main_json_before
    );
    // 带进来的都不是实例定义管的字段：实例状态不变
    assert_eq!(element(&status(&ctx), "cc-ly")["state"], state_before);

    // 再带一次：都已有，什么都不写
    let instance_before = snapshot_dir(&dir);
    let again = seed(&ctx, "cc-ly", "permissions,statusline,hooks,rules,mcp");
    assert!(again.ok, "{:?}", again.errors);
    assert_eq!(
        seed_states(&again),
        states(&[
            ("permissions", "exists"),
            ("statusline", "exists"),
            ("hooks", "exists"),
            ("rules", "exists"),
            ("mcp", "skipped"),
        ])
    );
    assert_eq!(snapshot_dir(&dir), instance_before);

    // 审计只记项与结果
    let runs = execute(&ctx, &request("runs:get", &["--run", "profiles-test"]));
    assert!(runs.ok, "{:?}", runs.errors);
    let text = serde_json::to_string(&runs.data).unwrap();
    assert!(text.contains("profiles:seed"), "{text}");
    for secret in [
        "literal-token-value",
        "main-secret-token",
        "echo done",
        "我的规则",
    ] {
        assert!(!text.contains(secret), "审计里出现了内容 {secret}");
    }
    std::env::remove_var("AM_TEST_SEED_CLAUDE_TOKEN");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn 默认环境的设置可一次性带进_codex_独立环境_不破坏接入_状态栏在_tui_下() {
    let (root, ctx) = temp_ctx("seed-codex");
    std::env::set_var("AM_TEST_SEED_CODEX_TOKEN", "seed-codex-value");
    let main_dir = ctx.home_dir.join(".codex");
    std::fs::create_dir_all(&main_dir).unwrap();
    std::fs::write(
        main_dir.join("config.toml"),
        r#"model = "gpt-5.5"
approval_policy = "on-request"
sandbox_mode = "workspace-write"

[sandbox_workspace_write]
network_access = true

[tui]
status_line = ["model", "cwd"]

[mcp_servers.fs]
command = "npx"
args = ["-y", "fs-mcp"]

[mcp_servers.gh]
command = "gh-mcp"
env = { GITHUB_TOKEN = "literal-token-value" }
"#,
    )
    .unwrap();
    std::fs::write(
        main_dir.join("hooks.json"),
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo codex"}]}]}}"#,
    )
    .unwrap();
    std::fs::write(main_dir.join("AGENTS.md"), "codex rules\n").unwrap();
    let main_before = snapshot_dir(&main_dir);

    assert!(save_definition(
        &ctx,
        "cx-work",
        r#"{"display_name":"Codex · work","bin":"codex","mcp_config":{"path":"~/.codex-work/config.toml","format":"toml"},"profile":{"base_agent":"codex","config_dir":"~/.codex-work","base_url":"https://api.example.com/v1","env_key":"AM_TEST_SEED_CODEX_TOKEN","model":"gpt-5.5"},"_delete":false,"_replace":true}"#
    ));
    let apply = apply_now(&ctx, "cx-work");
    assert!(apply.ok, "{:?}", apply.errors);
    let dir = ctx.home_dir.join(".codex-work");

    let env = seed(&ctx, "cx-work", "permissions,statusline,hooks,rules,mcp");
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        seed_states(&env),
        states(&[
            ("permissions", "copied"),
            ("statusline", "copied"),
            ("hooks", "copied"),
            ("rules", "copied"),
            ("mcp", "copied"),
        ]),
        "{}",
        env.data
    );
    let doc: toml::Table = std::fs::read_to_string(dir.join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(doc["approval_policy"].as_str(), Some("on-request"));
    assert_eq!(doc["sandbox_mode"].as_str(), Some("workspace-write"));
    assert_eq!(
        doc["sandbox_workspace_write"]["network_access"].as_bool(),
        Some(true)
    );
    assert_eq!(doc["tui"]["status_line"].as_array().map(Vec::len), Some(2));
    assert_eq!(doc["mcp_servers"]["fs"]["command"].as_str(), Some("npx"));
    assert!(
        doc["mcp_servers"].get("gh").is_none(),
        "含明文凭据的条目不带"
    );
    assert_eq!(
        doc["model_provider"].as_str(),
        Some("agent-manager"),
        "实例自己的接入原样保留"
    );
    let hooks: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("hooks.json")).unwrap()).unwrap();
    assert_eq!(
        hooks["hooks"]["Stop"][0]["hooks"][0]["command"],
        "echo codex"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("AGENTS.md")).unwrap(),
        "codex rules\n"
    );
    assert_eq!(snapshot_dir(&main_dir), main_before, "主配置不动");
    std::env::remove_var("AM_TEST_SEED_CODEX_TOKEN");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn 带设置的前提不满足时拒绝且零写_各项失败互不牵连() {
    let (root, ctx) = temp_ctx("seed-guard");
    std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
    std::fs::write(
        ctx.home_dir.join(".claude/settings.json"),
        r#"{"permissions":{"allow":["Bash(ls:*)"]}}"#,
    )
    .unwrap();
    std::fs::write(ctx.home_dir.join(".claude/CLAUDE.md"), "rules\n").unwrap();
    assert!(save_definition(
        &ctx,
        "cc-work",
        r#"{"display_name":"Claude Code · work","bin":"claude","profile":{"base_agent":"claude-code","config_dir":"~/.claude-work","model":"opus"},"_delete":false,"_replace":true}"#
    ));

    // 未知实例、坏参数、不安全的目录：拒绝，什么都不建
    assert!(!seed(&ctx, "nope", "rules").ok);
    assert!(!seed(&ctx, "cc-work", "env").ok);
    assert!(
        !execute(
            &ctx,
            &request("profiles:seed", &["--only", "profiles:cc-work"])
        )
        .ok
    );
    let _ = save_definition(
        &ctx,
        "cc-main",
        r#"{"display_name":"Claude Code · main","bin":"claude","profile":{"base_agent":"claude-code","config_dir":"~/.claude","model":"opus"},"_delete":false,"_replace":true}"#,
    );
    assert!(
        !seed(&ctx, "cc-main", "rules").ok,
        "不能把主配置目录当实例目录"
    );
    assert!(!ctx.home_dir.join(".claude-work").exists());

    // 目录还没建（官方账号 Codex 环境还没写过配置时就是这样）：由实例域建好（仅本人可访问）再带
    let dir = ctx.home_dir.join(".claude-work");
    let env = seed(&ctx, "cc-work", "rules");
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(seed_states(&env), states(&[("rules", "copied")]));
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
    std::fs::remove_file(dir.join("CLAUDE.md")).unwrap();

    // 实例 settings.json 损坏：权限项失败，规则照常带
    std::fs::write(dir.join("settings.json"), "{ not json").unwrap();
    let env = seed(&ctx, "cc-work", "permissions,rules");
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        seed_states(&env),
        states(&[("permissions", "failed"), ("rules", "copied")])
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("settings.json")).unwrap(),
        "{ not json",
        "损坏的文件原样不动"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap(),
        "rules\n"
    );

    // 主配置里没有的项：报「没有」，不写
    std::fs::write(dir.join("settings.json"), "{}\n").unwrap();
    let env = seed(&ctx, "cc-work", "statusline");
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(seed_states(&env), states(&[("statusline", "absent")]));
    assert_eq!(
        std::fs::read_to_string(dir.join("settings.json")).unwrap(),
        "{}\n"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn 密钥换了还没写进环境_状态标出凭据项_不提供以实际配置更新定义() {
    let (root, ctx) = temp_ctx("credential-stale");
    std::env::set_var("AM_TEST_STALE_TOKEN", "sk-old-value-1234567890");
    assert!(save_definition(
        &ctx,
        "cc-ly",
        &claude_profile("AM_TEST_STALE_TOKEN")
    ));
    let apply = apply_now(&ctx, "cc-ly");
    assert!(apply.ok, "{:?}", apply.errors);
    let el = element(&status(&ctx), "cc-ly").clone();
    let token = el["observed"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["field"] == "env.ANTHROPIC_AUTH_TOKEN")
        .cloned()
        .expect("有凭据字段");
    assert_eq!(token["purpose"], "credential");
    assert_eq!(token["variable"], "AM_TEST_STALE_TOKEN");
    assert_eq!(token["state"], "same");

    // 用户在密钥文件里换了值：环境里还是旧的
    std::env::set_var("AM_TEST_STALE_TOKEN", "sk-new-value-0987654321");
    let el = element(&status(&ctx), "cc-ly").clone();
    let text = serde_json::to_string(&el).unwrap();
    assert!(
        !text.contains("sk-old-value") && !text.contains("sk-new-value"),
        "不输出取值"
    );
    let token = el["observed"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["field"] == "env.ANTHROPIC_AUTH_TOKEN")
        .cloned()
        .unwrap();
    assert_eq!(token["state"], "differs");
    assert_eq!(token["purpose"], "credential");
    assert_eq!(el["state"], "drifted");
    let actions: Vec<&str> = el["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["id"].as_str())
        .collect();
    assert_eq!(actions, vec!["apply"], "只剩凭据不同：只给写入，不给采纳");

    // 写入后新值生效
    let apply = apply_now(&ctx, "cc-ly");
    assert!(apply.ok, "{:?}", apply.errors);
    assert_eq!(
        claude_settings(&ctx, ".claude-ly")["env"]["ANTHROPIC_AUTH_TOKEN"],
        "sk-new-value-0987654321"
    );
    std::env::remove_var("AM_TEST_STALE_TOKEN");
    let _ = std::fs::remove_dir_all(root);
}
