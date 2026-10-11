//! MCP 原生域：各助手配置文件里的实际条目即当前配置。
//!
//! 覆盖：只读 status（零写、脱敏、契约形状）、逐条 add/edit/enable/disable/remove/copy、
//! `--base` CAS、Codex 原位编辑保留未建模字段、Claude 停用快照（不迁移凭据、普通字面量
//! 原样保存）、旧服务库迁移、锁与审计、严格参数。全部在隔离 HOME 与数据目录里跑。

#[cfg(windows)]
mod common;

use agent_manager_engine::native_mcp::model::{digest_of, is_sha256_hex};
use agent_manager_engine::{execute, store, Context, Envelope, Request};
use fs2::FileExt;
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    let base = std::env::temp_dir().join(format!(
        "am-native-mcp-test-{}-{}-{:x}",
        std::process::id(),
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&base);
    #[cfg(windows)]
    common::private_fixture_root(&base);
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
        run_id: "mcp-test-run".to_string(),
    }
}

fn run(ctx: &Context, command: &str, args: &[&str]) -> Envelope {
    execute(ctx, &request(command, args))
}

fn errors(env: &Envelope) -> String {
    env.errors
        .iter()
        .map(|d| d.message.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}

fn warnings(env: &Envelope) -> String {
    env.warnings
        .iter()
        .map(|d| d.message.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}

fn apply(ctx: &Context, args: &[&str]) -> Envelope {
    run(ctx, "mcp:apply", args)
}

fn apply_ok(ctx: &Context, args: &[&str]) -> Envelope {
    let env = apply(ctx, args);
    assert!(env.ok, "{args:?} 应成功: {}", errors(&env));
    env
}

fn apply_err(ctx: &Context, args: &[&str]) -> String {
    let env = apply(ctx, args);
    assert!(!env.ok, "{args:?} 应被拒绝");
    errors(&env)
}

fn status(ctx: &Context) -> Value {
    let env = run(ctx, "mcp:status", &[]);
    assert!(env.ok, "{}", errors(&env));
    env.data
}

#[test]
fn preview_matches_add_and_edit_without_writing_and_rejects_stale_entry() {
    let (root, ctx) = temp_ctx("preview");
    let unsupported = run(
        &ctx,
        "mcp:preview",
        &[
            "--only",
            "mcp:example",
            "--agent",
            "codex",
            "--action",
            "disable",
            "--base",
            &"a".repeat(64),
        ],
    );
    assert!(!unsupported.ok);
    assert!(errors(&unsupported).contains("仅支持 add/edit/clean-markers/copy/remove"));
    assert!(!ctx.data_dir.exists());
    let spec = r#"{"command":"npx","args":["-y","example-mcp"]}"#;
    for (agent, path) in [
        ("claude-code", claude_path(&ctx)),
        ("codex", codex_path(&ctx)),
    ] {
        let args = [
            "--only",
            "mcp:example",
            "--agent",
            agent,
            "--action",
            "add",
            "--spec",
            spec,
        ];
        let before_db = db_bytes(&ctx);
        let preview = run(&ctx, "mcp:preview", &args);
        assert!(preview.ok, "{}", errors(&preview));
        let files = preview.data["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["path"], path.to_string_lossy().as_ref());
        assert_eq!(files[0]["exists"], false);
        assert_eq!(
            files[0]["original"],
            serde_json::Value::Null,
            "新文件没有原文"
        );
        let expected = files[0]["content"].as_str().unwrap();
        assert!(!path.exists(), "预览不得创建目标文件");
        assert_eq!(db_bytes(&ctx), before_db, "预览不得写库");
        apply_ok(&ctx, &args);
        assert_eq!(read(&path), expected, "预览必须与实际写入逐字节一致");
    }

    let original = read(&codex_path(&ctx));
    let b = base(&ctx, "example", "codex");
    let edit_spec = r#"{"command":"npx","args":["-y","example-mcp@2"]}"#;
    let args = [
        "--only",
        "mcp:example",
        "--agent",
        "codex",
        "--action",
        "edit",
        "--base",
        &b,
        "--spec",
        edit_spec,
    ];
    let preview = run(&ctx, "mcp:preview", &args);
    assert!(preview.ok, "{}", errors(&preview));
    let expected = preview.data["files"][0]["content"].as_str().unwrap();
    assert_eq!(
        preview.data["files"][0]["original"].as_str().unwrap(),
        original,
        "预览带上写入前的原文，界面据此只显示变化的部分"
    );
    assert_eq!(read(&codex_path(&ctx)), original);
    apply_ok(&ctx, &args);
    assert_eq!(read(&codex_path(&ctx)), expected);

    let latest_base = base(&ctx, "example", "codex");
    let current_args = [
        "--only",
        "mcp:example",
        "--agent",
        "codex",
        "--action",
        "edit",
        "--base",
        &latest_base,
        "--spec",
        edit_spec,
    ];
    let external = read(&codex_path(&ctx)).replace("example-mcp@2", "example-mcp@3");
    write_codex(&ctx, &external);
    let stale = run(&ctx, "mcp:preview", &current_args);
    assert!(!stale.ok);
    assert!(errors(&stale).contains("已在外部修改"));
    assert_eq!(read(&codex_path(&ctx)), external);
    let _ = std::fs::remove_dir_all(root);
}

fn element<'a>(data: &'a Value, server: &str) -> &'a Value {
    data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == format!("mcp:{server}"))
        .unwrap_or_else(|| panic!("缺少元素 mcp:{server}: {data}"))
}

fn cell(ctx: &Context, server: &str, agent: &str) -> Value {
    let data = status(ctx);
    element(&data, server)["observed"]["agents"][agent].clone()
}

fn base(ctx: &Context, server: &str, agent: &str) -> String {
    let c = cell(ctx, server, agent);
    c["base"]
        .as_str()
        .unwrap_or_else(|| panic!("{server}@{agent} 没有 base: {c}"))
        .to_string()
}

fn claude_path(ctx: &Context) -> PathBuf {
    ctx.home_dir.join(".claude.json")
}

fn codex_path(ctx: &Context) -> PathBuf {
    ctx.home_dir.join(".codex").join("config.toml")
}

fn write_claude(ctx: &Context, doc: Value) {
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    std::fs::write(
        claude_path(ctx),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
}

fn write_codex(ctx: &Context, text: &str) {
    std::fs::create_dir_all(codex_path(ctx).parent().unwrap()).unwrap();
    std::fs::write(codex_path(ctx), text).unwrap();
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn claude_entry(ctx: &Context, server: &str) -> Value {
    let doc: Value = serde_json::from_str(&read(&claude_path(ctx))).unwrap();
    doc["mcpServers"][server].clone()
}

fn codex_doc(ctx: &Context) -> toml::Value {
    toml::from_str(&read(&codex_path(ctx))).expect("Codex 配置必须仍是合法 TOML")
}

/// 选一个隔离 HOME 下的外部密钥文件（600）。
fn configure_secrets_file(ctx: &Context) -> PathBuf {
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let path = ctx.home_dir.join("secrets.env");
    std::fs::write(&path, "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let env = run(ctx, "secrets:source", &["--path", path.to_str().unwrap()]);
    assert!(env.ok, "{}", errors(&env));
    path
}

/// 数据库（含 WAL）里的全部字节，用来 grep 明文。
fn db_bytes(ctx: &Context) -> Vec<u8> {
    let mut out = Vec::new();
    if let Ok(dir) = std::fs::read_dir(&ctx.data_dir) {
        for entry in dir.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with("native-manager.db")
            {
                out.extend(std::fs::read(entry.path()).unwrap_or_default());
            }
        }
    }
    out
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

fn saved_rows(ctx: &Context) -> Vec<(String, String, String, String)> {
    let Some(conn) = store::open_readonly(ctx).unwrap() else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT server, agent, entry_json, resolve_json FROM mcp_saved ORDER BY server, agent",
    ) else {
        return Vec::new();
    };
    stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn op_details(ctx: &Context) -> Vec<(String, String)> {
    let Some(conn) = store::open_readonly(ctx).unwrap() else {
        return Vec::new();
    };
    let mut stmt = conn
        .prepare("SELECT operation_type, coalesce(detail_json, '') FROM operation_logs")
        .unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn load_contract_schema(rel_path: &str) -> Value {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let p = manifest.join("../").join(rel_path);
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap()
}

fn validate_json_schema(val: &Value, schema: &Value, root: &Value) {
    if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
        let name = r.strip_prefix("#/$defs/").unwrap();
        validate_json_schema(val, &root["$defs"][name], root);
        return;
    }
    if let Some(t) = schema.get("type") {
        let ok = |t: &str| match t {
            "object" => val.is_object(),
            "array" => val.is_array(),
            "string" => val.is_string(),
            "integer" => val.is_i64() || val.is_u64(),
            "boolean" => val.is_boolean(),
            "null" => val.is_null(),
            _ => true,
        };
        match t {
            Value::String(t) => assert!(ok(t), "期望 {t}，实际 {val}"),
            Value::Array(ts) => assert!(
                ts.iter().any(|t| ok(t.as_str().unwrap())),
                "{val} 不符合 {ts:?}"
            ),
            _ => {}
        }
    }
    if let Some(enums) = schema.get("enum").and_then(Value::as_array) {
        assert!(enums.contains(val), "{val} 不在 {enums:?}");
    }
    if let Some(obj) = val.as_object() {
        for req in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            assert!(obj.contains_key(req.as_str().unwrap()), "缺少 {req}: {val}");
        }
        if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
            let props = schema["properties"].as_object().unwrap();
            for key in obj.keys() {
                assert!(props.contains_key(key), "不允许的字段 {key}");
            }
        }
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (k, s) in props {
                if let Some(v) = obj.get(k) {
                    validate_json_schema(v, s, root);
                }
            }
        }
    }
    if let (Some(arr), Some(items)) = (val.as_array(), schema.get("items")) {
        for item in arr {
            validate_json_schema(item, items, root);
        }
    }
}

const CODEX_HANDWRITTEN: &str = r#"# 顶部注释
model = "o3"

# 关于 github 的说明
[mcp_servers.github]
command = "npx" # 行尾注释
args = [
  "-y",
  "gh-mcp",
]
cwd = "/work/repo"
enabled_tools = ["search", "read"]
startup_timeout_sec = 30

[mcp_servers.github.env]
GH_TOKEN = "plain-gh-token"

[mcp_servers.github.tools.search]
approval_mode = "approve"

# 关于 other 的说明
[mcp_servers.other]
command = "other-bin"

[profiles.fast]
model = "o4-mini"
"#;

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

/// 缺库缺文件：成功、零写；有实际条目时直接列出（无需导入），脱敏、给出 base，
/// 输出符合 status 契约；读取前后文件与数据目录都不变。
#[test]
fn status_lists_actual_entries_masked_zero_write_and_matches_contract() {
    let (root, ctx) = temp_ctx("status");
    let empty = status(&ctx);
    assert_eq!(empty["elements"], json!([]));
    let targets = empty["targets"].as_array().unwrap();
    assert_eq!(
        targets
            .iter()
            .map(|t| t["agent"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["claude-code", "codex"]
    );
    assert_eq!(targets[0]["disable_mode"], "snapshot");
    assert_eq!(targets[1]["disable_mode"], "field");
    assert!(!ctx.data_dir.exists(), "status 不得创建数据目录");
    assert!(!ctx.home_dir.exists(), "status 不得创建 HOME 下的任何东西");

    let secret = "sk-ant-api03-status0123456789abcdefghij";
    write_claude(
        &ctx,
        json!({
            "numStartups": 7,
            "mcpServers": {
                "fs": {"type": "stdio", "command": "npx", "args": ["-y", "fs-mcp"],
                       "env": {"API_KEY": secret, "REF": "${AM_STATUS_REF_VAR}"}, "alwaysLoad": true},
                "remote": {"type": "http", "url": "https://mcp.example.com/mcp",
                           "headers": {"Authorization": "Bearer inline-header-value", "X-Ref": "${AM_STATUS_HDR}"},
                           "headersHelper": "print-token --secret"}
            }
        }),
    );
    write_codex(&ctx, CODEX_HANDWRITTEN);
    let claude_before = read(&claude_path(&ctx));
    let codex_before = read(&codex_path(&ctx));

    let env = run(&ctx, "mcp:status", &[]);
    assert!(env.ok, "{}", errors(&env));
    let data = env.data.clone();
    let serialized = serde_json::to_string(&data).unwrap();
    for plaintext in [
        secret,
        "inline-header-value",
        "plain-gh-token",
        "print-token --secret",
    ] {
        assert!(
            !serialized.contains(plaintext),
            "status 不得输出明文 {plaintext}"
        );
    }

    let fs = &element(&data, "fs")["observed"]["agents"]["claude-code"];
    assert_eq!(fs["state"], "enabled");
    assert_eq!(fs["source"], "file");
    assert_eq!(fs["entry"]["env"]["API_KEY"], "<literal>");
    assert_eq!(fs["entry"]["env"]["REF"], "${AM_STATUS_REF_VAR}");
    assert_eq!(fs["extra"]["alwaysLoad"], true);
    assert_eq!(fs["vars"]["REF"]["placeholder"], "${AM_STATUS_REF_VAR}");
    assert!(is_sha256_hex(fs["base"].as_str().unwrap()));
    assert_eq!(fs["actions"], json!(["edit", "disable", "remove"]));
    assert_eq!(fs["copy_safe"], false);
    assert!(fs["copy_blocked_reason"].as_str().unwrap().contains("凭据"));

    let remote = &element(&data, "remote")["observed"]["agents"]["claude-code"];
    assert_eq!(remote["transport"], "remote");
    assert_eq!(remote["entry"]["headers"]["Authorization"], "<literal>");
    assert_eq!(remote["entry"]["headers"]["X-Ref"], "${AM_STATUS_HDR}");
    assert_eq!(remote["extra"]["headersHelper"], "<hidden>");

    let github = &element(&data, "github")["observed"]["agents"]["codex"];
    assert_eq!(github["entry"]["args"], json!(["-y", "gh-mcp"]));
    assert_eq!(github["entry"]["startup_timeout_sec"], 30);
    assert_eq!(github["extra"]["cwd"], "/work/repo");
    assert_eq!(github["extra"]["enabled_tools"], json!(["search", "read"]));
    assert_eq!(github["extra"]["tools"], "<hidden>");
    assert_eq!(element(&data, "github")["state"], "managed-ok");

    let schema = load_contract_schema("contract/engine-v1/status.schema.json");
    validate_json_schema(&data, &schema, &schema);
    let envelope_schema = load_contract_schema("contract/engine-v1/envelope.schema.json");
    validate_json_schema(
        &serde_json::to_value(&env).unwrap(),
        &envelope_schema,
        &envelope_schema,
    );

    assert_eq!(read(&claude_path(&ctx)), claude_before);
    assert_eq!(read(&codex_path(&ctx)), codex_before);
    assert!(!ctx.data_dir.exists(), "status 不得创建数据目录");
    let _ = std::fs::remove_dir_all(root);
}

/// 一个助手的文件坏了只影响它自己：报在 targets，另一个助手照常列出与操作。
#[test]
fn broken_target_is_reported_per_agent_and_blocks_only_that_agent() {
    let (root, ctx) = temp_ctx("broken-target");
    write_claude(&ctx, json!({"mcpServers": {"ok": {"command": "a"}}}));
    write_codex(&ctx, "[mcp_servers.x\ncommand = ");
    let data = status(&ctx);
    let codex = data["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["agent"] == "codex")
        .unwrap()
        .clone();
    assert!(
        codex["error"].as_str().unwrap().contains("解析失败"),
        "{codex}"
    );
    assert_eq!(
        element(&data, "ok")["observed"]["agents"]["claude-code"]["state"],
        "enabled"
    );
    let err = apply_err(
        &ctx,
        &[
            "--only",
            "mcp:new",
            "--agent",
            "codex",
            "--action",
            "add",
            "--spec",
            r#"{"command":"x"}"#,
        ],
    );
    assert!(err.contains("不可用"), "{err}");
    assert!(!ctx.data_dir.exists(), "预检拒绝不得建库");

    // 目标是符号链接：拒绝，不跟随
    #[cfg(unix)]
    {
        let real = ctx.home_dir.join("real.toml");
        std::fs::write(&real, "").unwrap();
        std::fs::remove_file(codex_path(&ctx)).unwrap();
        std::os::unix::fs::symlink(&real, codex_path(&ctx)).unwrap();
        let data = status(&ctx);
        let codex = data["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["agent"] == "codex")
            .unwrap()
            .clone();
        assert!(
            codex["error"].as_str().unwrap().contains("符号链接"),
            "{codex}"
        );
        apply_err(
            &ctx,
            &[
                "--only",
                "mcp:new",
                "--agent",
                "codex",
                "--action",
                "add",
                "--spec",
                r#"{"command":"x"}"#,
            ],
        );
        assert_eq!(read(&real), "", "符号链接指向的文件不得被写");
    }
    let _ = std::fs::remove_dir_all(root);
}

/// stdio 命令能否在本机解析：只做存在性判断；远程条目恒 null。
#[test]
fn status_reports_whether_the_stdio_command_exists() {
    let (root, ctx) = temp_ctx("command-availability");
    let bin_dir = ctx.home_dir.join(".local/bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    // Windows 上可执行的是 .cmd / .exe
    let file = if cfg!(windows) {
        "present-tool.cmd"
    } else {
        "present-tool"
    };
    std::fs::write(bin_dir.join(file), "#!/bin/sh\n").unwrap();
    write_claude(
        &ctx,
        json!({"mcpServers": {
            "with-command": {"command": "present-tool"},
            "missing-command": {"command": "definitely-not-installed-xyz"},
            "remote-one": {"type": "http", "url": "https://mcp.example.com/mcp"}
        }}),
    );
    let data = status(&ctx);
    let avail = |name: &str| {
        element(&data, name)["observed"]["agents"]["claude-code"]["command_available"].clone()
    };
    assert_eq!(avail("with-command"), true);
    assert_eq!(avail("missing-command"), false);
    assert_eq!(avail("remote-one"), Value::Null);
    let _ = std::fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// edit：只改已建模字段，未建模字段与其他内容原样保留
// ---------------------------------------------------------------------------

/// 必测 1：外部手写的 Codex 条目（含 cwd、enabled_tools、tools 子表、注释）直接可编辑；
/// 改 args 后未建模字段全部保留、TOML 可解析、其他表与注释逐字节保留。
#[test]
fn codex_handwritten_entry_edit_preserves_unmodeled_fields_and_other_tables() {
    let (root, ctx) = temp_ctx("codex-edit");
    write_codex(&ctx, CODEX_HANDWRITTEN);
    let b = base(&ctx, "github", "codex");
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:github",
            "--agent",
            "codex",
            "--action",
            "edit",
            "--base",
            &b,
            "--spec",
            r#"{"command":"npx","args":["-y","gh-mcp@2"],"env":{"GH_TOKEN":"<literal>"},"startup_timeout_sec":30}"#,
        ],
    );
    let text = read(&codex_path(&ctx));
    let doc = codex_doc(&ctx);
    let gh = &doc["mcp_servers"]["github"];
    assert_eq!(gh["args"].as_array().unwrap().len(), 2);
    assert_eq!(gh["args"][1].as_str(), Some("gh-mcp@2"));
    assert_eq!(gh["cwd"].as_str(), Some("/work/repo"));
    assert_eq!(gh["enabled_tools"].as_array().unwrap().len(), 2);
    assert_eq!(
        gh["tools"]["search"]["approval_mode"].as_str(),
        Some("approve")
    );
    assert_eq!(
        gh["env"]["GH_TOKEN"].as_str(),
        Some("plain-gh-token"),
        "<literal> 保留原值"
    );
    assert_eq!(gh["startup_timeout_sec"].as_integer(), Some(30));
    // 未改动的行与其他表逐字节保留
    for kept in [
        "# 顶部注释\nmodel = \"o3\"\n\n# 关于 github 的说明\n[mcp_servers.github]\ncommand = \"npx\" # 行尾注释\n",
        "cwd = \"/work/repo\"\nenabled_tools = [\"search\", \"read\"]\nstartup_timeout_sec = 30\n\n[mcp_servers.github.env]\nGH_TOKEN = \"plain-gh-token\"\n\n[mcp_servers.github.tools.search]\napproval_mode = \"approve\"\n\n# 关于 other 的说明\n[mcp_servers.other]\ncommand = \"other-bin\"\n\n[profiles.fast]\nmodel = \"o4-mini\"\n",
    ] {
        assert!(text.contains(kept), "应逐字节保留:\n{kept}\n实际:\n{text}");
    }
    // 编辑后 status 仍列出、base 已变化
    let after = base(&ctx, "github", "codex");
    assert_ne!(after, b);
    // 重复同样的编辑：字节稳定
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:github",
            "--agent",
            "codex",
            "--action",
            "edit",
            "--base",
            &after,
            "--spec",
            r#"{"command":"npx","args":["-y","gh-mcp@2"],"env":{"GH_TOKEN":"<literal>"},"startup_timeout_sec":30}"#,
        ],
    );
    assert_eq!(read(&codex_path(&ctx)), text, "内容不变的编辑不改字节");
    let _ = std::fs::remove_dir_all(root);
}

/// 行内表与点号键写法的条目同样原位编辑，其他条目不动。
#[test]
fn codex_inline_and_dotted_entries_are_edited_in_place() {
    let (root, ctx) = temp_ctx("codex-forms");
    let text = "[mcp_servers]\ninline = { command = \"a\", args = [\"1\"], cwd = \"/w\" }\nkeep = { command = \"k\" }\n";
    write_codex(&ctx, text);
    let b = base(&ctx, "inline", "codex");
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:inline",
            "--agent",
            "codex",
            "--action",
            "edit",
            "--base",
            &b,
            "--spec",
            r#"{"command":"a","args":["2"],"env":{"MODE":"fast"}}"#,
        ],
    );
    let after = read(&codex_path(&ctx));
    assert!(after.contains("keep = { command = \"k\" }\n"), "{after}");
    let doc = codex_doc(&ctx);
    assert_eq!(doc["mcp_servers"]["inline"]["cwd"].as_str(), Some("/w"));
    assert_eq!(
        doc["mcp_servers"]["inline"]["env"]["MODE"].as_str(),
        Some("fast")
    );
    assert_eq!(doc["mcp_servers"]["inline"]["args"][0].as_str(), Some("2"));

    let dotted = "other = 1\nmcp_servers.dot.command = \"d\"\nmcp_servers.dot.cwd = \"/x\"\n";
    write_codex(&ctx, dotted);
    let b = base(&ctx, "dot", "codex");
    apply_ok(
        &ctx,
        &[
            "--only", "mcp:dot", "--agent", "codex", "--action", "disable", "--base", &b,
        ],
    );
    let doc = codex_doc(&ctx);
    assert_eq!(doc["mcp_servers"]["dot"]["enabled"].as_bool(), Some(false));
    assert_eq!(doc["mcp_servers"]["dot"]["cwd"].as_str(), Some("/x"));
    assert!(read(&codex_path(&ctx))
        .starts_with("other = 1\nmcp_servers.dot.command = \"d\"\nmcp_servers.dot.cwd = \"/x\"\n"));
    let _ = std::fs::remove_dir_all(root);
}

/// Claude 条目编辑：未建模字段（alwaysLoad、type、oauth）与键顺序保留；文件其余部分不变；
/// 新写的普通字面量可以写入，疑似凭据的字面量被拒绝并提示改用 ${VAR}。
#[test]
fn claude_edit_keeps_unmodeled_fields_and_rejects_suspected_secret_literals() {
    let (root, ctx) = temp_ctx("claude-edit");
    write_claude(
        &ctx,
        json!({"numStartups": 3, "projects": {"/p": {"x": 1}}, "mcpServers": {
            "fs": {"type": "stdio", "command": "npx", "args": ["-y", "fs"], "env": {"KEEP": "orig"}, "alwaysLoad": true},
            "other": {"command": "o"}
        }}),
    );
    let b = base(&ctx, "fs", "claude-code");
    let err = apply_err(
        &ctx,
        &[
            "--only",
            "mcp:fs",
            "--agent",
            "claude-code",
            "--action",
            "edit",
            "--base",
            &b,
            "--spec",
            r#"{"command":"npx","args":[],"env":{"TOKEN":"sk-proj-abcdefghijklmnopqrstuvwxyz0123"}}"#,
        ],
    );
    assert!(err.contains("疑似凭据") && err.contains("${VAR}"), "{err}");
    assert!(!err.contains("sk-proj"), "错误不得回显取值: {err}");

    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:fs",
            "--agent",
            "claude-code",
            "--action",
            "edit",
            "--base",
            &b,
            "--spec",
            r#"{"command":"uvx","args":["fs-server"],"env":{"KEEP":"<literal>","NODE_ENV":"production"}}"#,
        ],
    );
    let doc: Value = serde_json::from_str(&read(&claude_path(&ctx))).unwrap();
    let fs = &doc["mcpServers"]["fs"];
    assert_eq!(fs["command"], "uvx");
    assert_eq!(fs["args"], json!(["fs-server"]));
    assert_eq!(fs["env"], json!({"KEEP": "orig", "NODE_ENV": "production"}));
    assert_eq!(fs["alwaysLoad"], true);
    assert_eq!(fs["type"], "stdio");
    assert_eq!(
        fs.as_object().unwrap().keys().collect::<Vec<_>>(),
        vec!["type", "command", "args", "env", "alwaysLoad"],
        "键顺序保留"
    );
    assert_eq!(doc["numStartups"], 3);
    assert_eq!(doc["projects"], json!({"/p": {"x": 1}}));
    assert_eq!(doc["mcpServers"]["other"], json!({"command": "o"}));
    assert!(!read(&claude_path(&ctx)).contains("_MANAGED_BY"));
    let _ = std::fs::remove_dir_all(root);
}

/// 必测 2：旧 base 编辑被拒——文件字节与数据库都不变，报错含「已在外部修改」；
/// 预检之后、写入之前文件被外部改动（直接调用 apply 绕过预检）同样被拒。
#[test]
fn stale_base_is_rejected_without_touching_file_or_database() {
    let (root, ctx) = temp_ctx("stale");
    write_codex(&ctx, CODEX_HANDWRITTEN);
    let old_base = base(&ctx, "github", "codex");
    let external = CODEX_HANDWRITTEN.replace("gh-mcp", "gh-mcp-external");
    write_codex(&ctx, &external);
    let err = apply_err(
        &ctx,
        &[
            "--only",
            "mcp:github",
            "--agent",
            "codex",
            "--action",
            "edit",
            "--base",
            &old_base,
            "--spec",
            r#"{"command":"npx","args":["mine"]}"#,
        ],
    );
    assert!(err.contains("已在外部修改"), "{err}");
    assert_eq!(read(&codex_path(&ctx)), external);
    assert!(!ctx.data_dir.exists(), "被拒绝的写入不得建库");

    // 同一 base 用在别的条目或别的助手上也不行
    let other = base(&ctx, "other", "codex");
    let err = apply_err(
        &ctx,
        &[
            "--only",
            "mcp:github",
            "--agent",
            "codex",
            "--action",
            "disable",
            "--base",
            &other,
        ],
    );
    assert!(err.contains("已在外部修改"), "{err}");

    // 绕过预检直接调用 apply：锁内重读后照样拒绝，数据库里没有任何 MCP 写入记录
    let fresh = base(&ctx, "github", "codex");
    write_codex(&ctx, CODEX_HANDWRITTEN);
    let direct = agent_manager_engine::native_mcp::apply::apply(
        &ctx,
        &request(
            "mcp:apply",
            &[
                "--only",
                "mcp:github",
                "--agent",
                "codex",
                "--action",
                "disable",
                "--base",
                &fresh,
            ],
        ),
    );
    let err = format!("{:#}", direct.expect_err("应拒绝"));
    assert!(err.contains("已在外部修改"), "{err}");
    assert_eq!(read(&codex_path(&ctx)), CODEX_HANDWRITTEN);
    assert!(
        op_details(&ctx)
            .iter()
            .all(|(op, _)| !op.starts_with("mcp:")),
        "被拒绝的写入不得留下操作记录"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// 必测 7：多个助手里的同名条目内容不同——分别展示、各有 base，编辑一个另一个字节不变。
#[test]
fn same_name_entries_in_multiple_agents_are_independent() {
    let (root, ctx) = temp_ctx("multi-agent");
    write_claude(
        &ctx,
        json!({"mcpServers": {"shared": {"command": "npx", "args": ["claude-side"]}}}),
    );
    write_codex(
        &ctx,
        "[mcp_servers.shared]\ncommand = \"npx\"\nargs = [\"codex-side\"]\n",
    );
    let data = status(&ctx);
    let el = element(&data, "shared");
    let c = &el["observed"]["agents"]["claude-code"];
    let x = &el["observed"]["agents"]["codex"];
    assert_eq!(c["entry"]["args"], json!(["claude-side"]));
    assert_eq!(x["entry"]["args"], json!(["codex-side"]));
    assert_ne!(c["base"], x["base"]);

    let claude_before = read(&claude_path(&ctx));
    let err = apply_err(
        &ctx,
        &[
            "--only",
            "mcp:shared",
            "--agent",
            "codex",
            "--action",
            "edit",
            "--base",
            c["base"].as_str().unwrap(),
            "--spec",
            r#"{"command":"npx","args":["x"]}"#,
        ],
    );
    assert!(
        err.contains("已在外部修改"),
        "别的助手的 base 不能用: {err}"
    );
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:shared",
            "--agent",
            "codex",
            "--action",
            "edit",
            "--base",
            x["base"].as_str().unwrap(),
            "--spec",
            r#"{"command":"npx","args":["codex-new"]}"#,
        ],
    );
    assert_eq!(
        read(&claude_path(&ctx)),
        claude_before,
        "Claude 文件不得被动"
    );
    assert_eq!(
        codex_doc(&ctx)["mcp_servers"]["shared"]["args"][0].as_str(),
        Some("codex-new")
    );
    let _ = std::fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 启停
// ---------------------------------------------------------------------------

/// 必测 3：Codex 停用写官方 `enabled = false`，条目仍在；再启用写 `enabled = true`。
#[test]
fn codex_disable_uses_official_enabled_field_and_keeps_entry() {
    let (root, ctx) = temp_ctx("codex-toggle");
    write_codex(&ctx, CODEX_HANDWRITTEN);
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:github",
            "--agent",
            "codex",
            "--action",
            "disable",
            "--base",
            &base(&ctx, "github", "codex"),
        ],
    );
    let doc = codex_doc(&ctx);
    assert_eq!(
        doc["mcp_servers"]["github"]["enabled"].as_bool(),
        Some(false)
    );
    assert_eq!(
        doc["mcp_servers"]["github"]["cwd"].as_str(),
        Some("/work/repo")
    );
    let c = cell(&ctx, "github", "codex");
    assert_eq!(c["state"], "disabled");
    assert_eq!(c["source"], "file");
    assert_eq!(c["actions"], json!(["edit", "enable", "remove"]));
    assert_eq!(c["copy_safe"], false);
    assert!(saved_rows(&ctx).is_empty(), "Codex 停用不存快照");
    let text = read(&codex_path(&ctx));
    assert!(text.contains("# 关于 other 的说明\n[mcp_servers.other]\ncommand = \"other-bin\"\n"));

    let err = apply_err(
        &ctx,
        &[
            "--only",
            "mcp:github",
            "--agent",
            "codex",
            "--action",
            "disable",
            "--base",
            &base(&ctx, "github", "codex"),
        ],
    );
    assert!(err.contains("已处于停用"), "{err}");
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:github",
            "--agent",
            "codex",
            "--action",
            "enable",
            "--base",
            &base(&ctx, "github", "codex"),
        ],
    );
    assert!(
        codex_doc(&ctx)["mcp_servers"]["github"]
            .get("enabled")
            .is_none(),
        "启用是去掉 enabled = false，不写原来没有的 enabled = true"
    );
    assert_eq!(cell(&ctx, "github", "codex")["state"], "enabled");
    let _ = std::fs::remove_dir_all(root);
}

/// 必测 4（及裁决第 10 条两类）：Claude 停用 = 移出条目 + 快照入库。
/// 疑似凭据的 env 字面量与全部 header 字面量转存到外部密钥文件、快照里只剩 ${VAR}；
/// Disabling existing owned references never adopts or overwrites credentials.
#[test]
fn claude_disable_snapshots_owned_references_and_enable_restores_identically() {
    let (root, ctx) = temp_ctx("claude-snapshot");
    let secrets_file = configure_secrets_file(&ctx);
    let original_fs = json!({"type":"stdio","command":"npx","args":["-y","fs"],
        "env":{"AM_T4_API_KEY":"${AM_T4_API_KEY}","NODE_ENV":"production"},"alwaysLoad":true});
    let original_remote = json!({"type":"http","url":"https://mcp.example.com/mcp",
        "headers":{"Authorization":"${AM_T4_HDR}"}});
    write_claude(
        &ctx,
        json!({"numStartups":1,"mcpServers":{"fs":original_fs,"remote":original_remote,"other":{"command":"o"}}}),
    );
    let before_secret = read(&secrets_file);
    for name in ["fs", "remote"] {
        apply_ok(
            &ctx,
            &[
                "--only",
                &format!("mcp:{name}"),
                "--agent",
                "claude-code",
                "--action",
                "disable",
                "--base",
                &base(&ctx, name, "claude-code"),
            ],
        );
    }
    assert_eq!(
        read(&secrets_file),
        before_secret,
        "disable never adopts keys"
    );
    let doc: Value = serde_json::from_str(&read(&claude_path(&ctx))).unwrap();
    assert!(doc["mcpServers"].get("fs").is_none());
    assert!(doc["mcpServers"].get("remote").is_none());
    assert_eq!(doc["mcpServers"]["other"], json!({"command":"o"}));
    assert_eq!(doc["numStartups"], 1);
    let rows = saved_rows(&ctx);
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert_eq!(row.3, r#"{"env":[],"headers":[]}"#);
    }
    let c = cell(&ctx, "fs", "claude-code");
    assert_eq!(c["entry"]["env"]["AM_T4_API_KEY"], "${AM_T4_API_KEY}");
    assert_eq!(c["actions"], json!(["enable", "remove"]));
    assert_eq!(c["copy_safe"], false);
    for name in ["fs", "remote"] {
        apply_ok(
            &ctx,
            &[
                "--only",
                &format!("mcp:{name}"),
                "--agent",
                "claude-code",
                "--action",
                "enable",
                "--base",
                &base(&ctx, name, "claude-code"),
            ],
        );
    }
    assert_eq!(claude_entry(&ctx, "fs"), original_fs);
    assert_eq!(claude_entry(&ctx, "remote"), original_remote);
    assert!(saved_rows(&ctx).is_empty());
    assert_eq!(read(&secrets_file), before_secret);
    let _ = std::fs::remove_dir_all(root);
}

/// Even a configured source is never silently used to migrate literal credentials.
#[test]
fn claude_disable_with_configured_secrets_refuses_literal_adoption_without_writes() {
    let (root, ctx) = temp_ctx("claude-no-adoption");
    let path = configure_secrets_file(&ctx);
    std::fs::write(&path, "ACCOUNT_A_KEY=FAKE_ACCOUNT_A_VALUE\n").unwrap();
    write_claude(
        &ctx,
        json!({"mcpServers":{"literal":{"command":"a","env":{"ACCOUNT_B_KEY":"FAKE_ACCOUNT_B_VALUE"}},"header":{"url":"https://mcp.example.com/mcp","headers":{"X-Team":"platform"}}}}),
    );
    let before_file = read(&claude_path(&ctx));
    let before_secret = read(&path);
    let before_db = db_bytes(&ctx);
    for name in ["literal", "header"] {
        let only = format!("mcp:{name}");
        let current_base = base(&ctx, name, "claude-code");
        let args = [
            "--only",
            only.as_str(),
            "--agent",
            "claude-code",
            "--action",
            "disable",
            "--base",
            current_base.as_str(),
        ];
        for command in ["mcp:preview", "mcp:apply"] {
            let result = run(&ctx, command, &args);
            assert!(!result.ok);
            assert!(
                command == "mcp:preview" || errors(&result).contains("不会自动迁移"),
                "{}",
                errors(&result)
            );
            assert!(!errors(&result).contains("FAKE_ACCOUNT_B_VALUE"));
            assert_eq!(read(&claude_path(&ctx)), before_file);
            assert_eq!(read(&path), before_secret);
            assert_eq!(db_bytes(&ctx), before_db);
        }
    }
    let _ = std::fs::remove_dir_all(root);
}

/// 裁决第 10 条的另一类：未选密钥文件时，含疑似凭据字面量（或任何 header 字面量）的
/// 条目拒绝停用，文件与库不变；只有普通字面量的条目可以直接停用。
#[test]
fn claude_disable_without_secrets_file_refuses_only_when_secrets_must_move() {
    let (root, ctx) = temp_ctx("claude-no-secrets");
    write_claude(
        &ctx,
        json!({"mcpServers": {
            "secret-env": {"command": "a", "env": {"AM_T6_KEY": "ghp_abcdefghijklmnopqrstuvwxyz0123"}},
            // 取值没有可识别的厂商前缀，但变量名像凭据：同样必须转存，不能以明文进快照
            "named-token": {"command": "c", "env": {"GITHUB_TOKEN": "custom-format-9f3a7c21", "NODE_ENV": "production"}},
            "plain-header": {"type": "http", "url": "https://x.example.com/mcp", "headers": {"X-Team": "platform"}},
            "plain-env": {"command": "b", "env": {"NODE_ENV": "production", "LOG_LEVEL": "debug"}}
        }}),
    );
    let before = read(&claude_path(&ctx));
    for name in ["secret-env", "named-token", "plain-header"] {
        let err = apply_err(
            &ctx,
            &[
                "--only",
                &format!("mcp:{name}"),
                "--agent",
                "claude-code",
                "--action",
                "disable",
                "--base",
                &base(&ctx, name, "claude-code"),
            ],
        );
        assert!(err.contains("不会自动迁移"), "{err}");
        assert!(!err.contains("ghp_"), "错误不得回显取值: {err}");
        assert!(!err.contains("custom-format"), "错误不得回显取值: {err}");
        assert_eq!(read(&claude_path(&ctx)), before, "被拒绝时文件不变");
    }
    assert!(!ctx.data_dir.exists(), "被拒绝时不建库");

    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:plain-env",
            "--agent",
            "claude-code",
            "--action",
            "disable",
            "--base",
            &base(&ctx, "plain-env", "claude-code"),
        ],
    );
    let rows = saved_rows(&ctx);
    assert_eq!(rows.len(), 1);
    let entry: Value = serde_json::from_str(&rows[0].2).unwrap();
    assert_eq!(
        entry["env"],
        json!({"NODE_ENV": "production", "LOG_LEVEL": "debug"})
    );
    assert_eq!(rows[0].3, r#"{"env":[],"headers":[]}"#);
    let _ = std::fs::remove_dir_all(root);
}

/// 快照写回的前提：目标文件里没有同名条目；有则拒绝。两者并存时 remove 一并删除。
#[test]
fn snapshot_enable_is_refused_when_same_name_entry_exists() {
    let (root, ctx) = temp_ctx("snapshot-conflict");
    write_claude(&ctx, json!({"mcpServers": {"x": {"command": "old"}}}));
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:x",
            "--agent",
            "claude-code",
            "--action",
            "disable",
            "--base",
            &base(&ctx, "x", "claude-code"),
        ],
    );
    let snapshot_base = base(&ctx, "x", "claude-code");
    // 用户在外部又手工加回了同名条目
    write_claude(&ctx, json!({"mcpServers": {"x": {"command": "new"}}}));
    let c = cell(&ctx, "x", "claude-code");
    assert_eq!(c["source"], "file");
    assert_eq!(c["has_saved"], true);
    let err = apply_err(
        &ctx,
        &[
            "--only",
            "mcp:x",
            "--agent",
            "claude-code",
            "--action",
            "enable",
            "--base",
            &snapshot_base,
        ],
    );
    assert!(err.contains("已存在同名条目"), "{err}");
    assert_eq!(claude_entry(&ctx, "x"), json!({"command": "new"}));

    // 新增同名：文件已有 → 拒绝
    let err = apply_err(
        &ctx,
        &[
            "--only",
            "mcp:x",
            "--agent",
            "claude-code",
            "--action",
            "add",
            "--spec",
            r#"{"command":"z"}"#,
        ],
    );
    assert!(err.contains("已有名为 x"), "{err}");

    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:x",
            "--agent",
            "claude-code",
            "--action",
            "remove",
            "--base",
            c["base"].as_str().unwrap(),
        ],
    );
    assert!(claude_entry(&ctx, "x").is_null());
    assert!(saved_rows(&ctx).is_empty(), "连同停用快照一并删除");
    let _ = std::fs::remove_dir_all(root);
}

/// OpenCode（已配置的助手）用官方 enabled 字段启停。
#[test]
fn opencode_target_toggles_official_enabled_field() {
    let (root, ctx) = temp_ctx("opencode");
    let set = run(
        &ctx,
        "policy:set",
        &[
            "--layer",
            "local",
            "--key",
            "agents.definitions.opencode",
            "--value",
            r#"{"display_name":"OpenCode","bin":"opencode","providers":{"native":{}},"preferred_provider":"native","mcp_config":{"path":"~/.config/opencode/opencode.json","format":"opencode"}}"#,
        ],
    );
    assert!(set.ok, "{}", errors(&set));
    let path = ctx.home_dir.join(".config/opencode/opencode.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"$schema":"https://opencode.ai/config.json","mcp":{"oc":{"type":"local","command":["npx","-y","oc"],"cwd":"/w"}}}"#,
    )
    .unwrap();
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:oc",
            "--agent",
            "opencode",
            "--action",
            "disable",
            "--base",
            &base(&ctx, "oc", "opencode"),
        ],
    );
    let doc: Value = serde_json::from_str(&read(&path)).unwrap();
    assert_eq!(doc["mcp"]["oc"]["enabled"], false);
    assert_eq!(doc["mcp"]["oc"]["cwd"], "/w");
    assert_eq!(doc["$schema"], "https://opencode.ai/config.json");
    assert_eq!(cell(&ctx, "oc", "opencode")["state"], "disabled");
    let _ = std::fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// add / copy / remove / forget
// ---------------------------------------------------------------------------

/// 新增直接写入所选助手：三种客户端都只有官方字段、没有任何管理标记；
/// `${VAR}` env 按现有规则解析写入；header 以客户端原生引用形式写入。
#[test]
fn add_writes_official_fields_only_for_all_clients() {
    let (root, ctx) = temp_ctx("add");
    let source = configure_secrets_file(&ctx);
    let mut values = String::new();
    for suffix in ["CLAUDE", "CODEX", "OPENCODE"] {
        values.push_str(&format!(
            "AM_T11_TOKEN_{suffix}=resolved-t11-value\nAM_T11_HDR_{suffix}=hdr-t11-value\n"
        ));
    }
    std::fs::write(&source, values).unwrap();
    let set = run(
        &ctx,
        "policy:set",
        &[
            "--layer",
            "local",
            "--key",
            "agents.definitions.opencode",
            "--value",
            r#"{"display_name":"OpenCode","bin":"opencode","providers":{"native":{}},"preferred_provider":"native","mcp_config":{"path":"~/.config/opencode/opencode.json","format":"opencode"}}"#,
        ],
    );
    assert!(set.ok);
    for (agent, suffix) in [
        ("claude-code", "CLAUDE"),
        ("codex", "CODEX"),
        ("opencode", "OPENCODE"),
    ] {
        let stdio_spec = format!(
            r#"{{"command":"npx","args":["-y","pkg"],"env":{{"TOKEN":"${{AM_T11_TOKEN_{suffix}}}","MODE":"fast"}}}}"#
        );
        let remote_spec = format!(
            r#"{{"url":"https://mcp.example.com/mcp","headers":{{"Authorization":"${{AM_T11_HDR_{suffix}}}"}}}}"#
        );
        apply_ok(
            &ctx,
            &[
                "--only",
                "mcp:stdio1",
                "--agent",
                agent,
                "--action",
                "add",
                "--spec",
                &stdio_spec,
            ],
        );
        apply_ok(
            &ctx,
            &[
                "--only",
                "mcp:remote1",
                "--agent",
                agent,
                "--action",
                "add",
                "--spec",
                &remote_spec,
            ],
        );
    }
    let claude = read(&claude_path(&ctx));
    let codex = read(&codex_path(&ctx));
    let opencode = read(&ctx.home_dir.join(".config/opencode/opencode.json"));
    for text in [&claude, &codex, &opencode] {
        assert!(
            !text.contains("_MANAGED_BY") && !text.contains("agent-manager"),
            "{text}"
        );
        assert!(!text.contains("hdr-t11-value"), "header 只写引用: {text}");
    }
    assert_eq!(
        claude_entry(&ctx, "stdio1")["env"]["TOKEN"],
        "resolved-t11-value"
    );
    assert_eq!(
        claude_entry(&ctx, "remote1")["headers"]["Authorization"],
        "${AM_T11_HDR_CLAUDE}"
    );
    assert_eq!(claude_entry(&ctx, "remote1")["type"], "http");
    let doc = codex_doc(&ctx);
    assert_eq!(
        doc["mcp_servers"]["stdio1"]["env"]["MODE"].as_str(),
        Some("fast")
    );
    assert_eq!(
        doc["mcp_servers"]["remote1"]["env_http_headers"]["Authorization"].as_str(),
        Some("AM_T11_HDR_CODEX")
    );
    let oc: Value = serde_json::from_str(&opencode).unwrap();
    // Windows 上新增的 npx 条目包成 cmd /c（Claude Code /doctor 的要求）
    let expected = if cfg!(windows) {
        json!(["cmd", "/c", "npx", "-y", "pkg"])
    } else {
        json!(["npx", "-y", "pkg"])
    };
    assert_eq!(oc["mcp"]["stdio1"]["command"], expected);
    assert_eq!(
        oc["mcp"]["remote1"]["headers"]["Authorization"],
        "{env:AM_T11_HDR_OPENCODE}"
    );

    // 反例：同名已存在、url 不在白名单、缺变量、疑似凭据字面量、超时只适用于 Codex
    for (spec, agent, expect) in [
        (r#"{"command":"x"}"#, "codex", "已有名为 stdio1"),
        (
            r#"{"url":"http://example.com/mcp"}"#,
            "claude-code",
            "服务地址不合法",
        ),
        (
            r#"{"command":"x","env":{"K":"${AM_T11_MISSING}"}}"#,
            "claude-code",
            "凭证变量未配置",
        ),
        (
            r#"{"command":"x","env":{"K":"AKIAABCDEFGHIJKLMNOP"}}"#,
            "claude-code",
            "疑似凭据",
        ),
        (
            r#"{"command":"x","startup_timeout_sec":5}"#,
            "claude-code",
            "只适用于 Codex",
        ),
        (r#"{"command":"x","extra":1}"#, "claude-code", "不支持字段"),
    ] {
        let name = if expect.contains("stdio1") {
            "stdio1"
        } else {
            "neg"
        };
        let err = apply_err(
            &ctx,
            &[
                "--only",
                &format!("mcp:{name}"),
                "--agent",
                agent,
                "--action",
                "add",
                "--spec",
                spec,
            ],
        );
        assert!(err.contains(expect), "{spec}: {err}");
        assert!(
            !err.contains("AKIAABCDEFGHIJKLMNOP"),
            "错误不得回显取值: {err}"
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

/// 必测 5：复制 Claude → Codex：字段按目标 codec 转换；目标同名默认拒绝；
/// 显式替换需给出目标条目的 base；未建模字段不复制、在 warnings 里列出。
#[test]
fn copy_claude_to_codex_converts_fields_and_guards_existing_target() {
    let (root, ctx) = temp_ctx("copy");
    write_claude(
        &ctx,
        json!({"mcpServers": {
            "fs": {"type": "stdio", "command": "npx", "args": ["-y", "fs"],
                   "env": {"LIT": "literal-value"}, "alwaysLoad": true},
            "remote": {"type": "http", "url": "https://mcp.example.com/mcp",
                       "headers": {"X-Lit": "static-value"}},
            "ws": {"type": "ws", "url": "wss://mcp.example.com/socket"}
        }}),
    );
    write_codex(&ctx, "# 用户的\n[mcp_servers.keep]\ncommand = \"k\"\n");
    let source_base = base(&ctx, "fs", "claude-code");
    let copy_args = [
        "--only",
        "mcp:fs",
        "--agent",
        "claude-code",
        "--to",
        "codex",
        "--action",
        "copy",
        "--base",
        source_base.as_str(),
    ];
    let before = read(&codex_path(&ctx));
    let before_db = db_bytes(&ctx);
    let preview = run(&ctx, "mcp:preview", &copy_args);
    assert!(preview.ok, "{}", errors(&preview));
    let candidate = preview.data["files"][0]["content"].as_str().unwrap();
    assert!(candidate.contains("[mcp_servers.fs]"));
    assert_eq!(read(&codex_path(&ctx)), before);
    assert_eq!(db_bytes(&ctx), before_db);
    let env = apply_ok(&ctx, &copy_args);
    assert_eq!(read(&codex_path(&ctx)), candidate);
    assert!(
        warnings(&env).contains("alwaysLoad"),
        "未复制的字段要说明: {}",
        warnings(&env)
    );
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:remote",
            "--agent",
            "claude-code",
            "--to",
            "codex",
            "--action",
            "copy",
            "--base",
            &base(&ctx, "remote", "claude-code"),
        ],
    );
    let doc = codex_doc(&ctx);
    let fs = &doc["mcp_servers"]["fs"];
    assert_eq!(
        fs["command"].as_str(),
        Some(if cfg!(windows) { "cmd" } else { "npx" })
    );
    assert_eq!(fs["env"]["LIT"].as_str(), Some("literal-value"));
    assert!(fs["env"].get("REF").is_none());
    assert!(fs.get("alwaysLoad").is_none());
    let remote = &doc["mcp_servers"]["remote"];
    assert_eq!(remote["url"].as_str(), Some("https://mcp.example.com/mcp"));
    assert!(remote.get("env_http_headers").is_none());
    assert_eq!(
        remote["http_headers"]["X-Lit"].as_str(),
        Some("static-value")
    );
    assert!(read(&codex_path(&ctx)).starts_with("# 用户的\n[mcp_servers.keep]\ncommand = \"k\"\n"));

    let err = apply_err(
        &ctx,
        &[
            "--only",
            "mcp:ws",
            "--agent",
            "claude-code",
            "--to",
            "codex",
            "--action",
            "copy",
            "--base",
            &base(&ctx, "ws", "claude-code"),
        ],
    );
    assert!(err.contains("streamable HTTP"), "{err}");

    // 目标同名：默认拒绝；--replace 必须带对的 --to-base
    let src = base(&ctx, "fs", "claude-code");
    let conflict = run(
        &ctx,
        "mcp:preview",
        &[
            "--only",
            "mcp:fs",
            "--agent",
            "claude-code",
            "--to",
            "codex",
            "--action",
            "copy",
            "--base",
            &src,
        ],
    );
    assert!(!conflict.ok);
    assert!(errors(&conflict).contains("已有同名条目"));
    let err = apply_err(
        &ctx,
        &[
            "--only",
            "mcp:fs",
            "--agent",
            "claude-code",
            "--to",
            "codex",
            "--action",
            "copy",
            "--base",
            &src,
        ],
    );
    assert!(err.contains("已有同名条目"), "{err}");
    let err = apply_err(
        &ctx,
        &[
            "--only",
            "mcp:fs",
            "--agent",
            "claude-code",
            "--to",
            "codex",
            "--action",
            "copy",
            "--base",
            &src,
            "--replace",
            "--to-base",
            &src,
        ],
    );
    assert!(err.contains("已在外部修改"), "{err}");
    let target_base = base(&ctx, "fs", "codex");
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:fs",
            "--agent",
            "claude-code",
            "--to",
            "codex",
            "--action",
            "copy",
            "--base",
            &src,
            "--replace",
            "--to-base",
            &target_base,
        ],
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn resolved_mcp_env_references_keep_durable_account_ownership_after_rendering() {
    let (root, ctx) = temp_ctx("mcp-durable-ownership");
    let source = configure_secrets_file(&ctx);
    std::fs::write(&source, "YOUR_CLAIMED_A=YOUR_FAKE_CLAIMED_A_VALUE\n").unwrap();
    write_claude(&ctx, json!({"mcpServers":{}}));
    write_codex(&ctx, "# B stays intact\n");
    let spec = r#"{"command":"mcp","env":{"TOKEN":"${YOUR_CLAIMED_A}"}}"#;
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:a",
            "--agent",
            "claude-code",
            "--action",
            "add",
            "--spec",
            spec,
        ],
    );
    assert_eq!(
        claude_entry(&ctx, "a")["env"]["TOKEN"],
        "YOUR_FAKE_CLAIMED_A_VALUE"
    );
    assert!(
        !read(&claude_path(&ctx)).contains("${YOUR_CLAIMED_A}"),
        "reference is absent from native file"
    );
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let owner: String = conn
        .query_row(
            "SELECT account FROM secret_variable_owners WHERE name='YOUR_CLAIMED_A'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(owner, "claude-code");
    drop(conn);
    let before_a = read(&claude_path(&ctx));
    let before_b = read(&codex_path(&ctx));
    let before_source = read(&source);
    let before_db = db_bytes(&ctx);
    for command in ["mcp:preview", "mcp:apply"] {
        let result = run(
            &ctx,
            command,
            &[
                "--only", "mcp:b", "--agent", "codex", "--action", "add", "--spec", spec,
            ],
        );
        assert!(
            !result.ok,
            "B cannot borrow A's key after its reference was rendered"
        );
        assert_eq!(read(&claude_path(&ctx)), before_a);
        assert_eq!(read(&codex_path(&ctx)), before_b);
        assert_eq!(read(&source), before_source);
        assert_eq!(db_bytes(&ctx), before_db);
    }
    // An edit by the same owner is still allowed.
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:a",
            "--agent",
            "claude-code",
            "--action",
            "edit",
            "--base",
            &base(&ctx, "a", "claude-code"),
            "--spec",
            r#"{"command":"mcp2","env":{"TOKEN":"${YOUR_CLAIMED_A}"}}"#,
        ],
    );
    assert_eq!(claude_entry(&ctx, "a")["command"], "mcp2");
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:a",
            "--agent",
            "claude-code",
            "--action",
            "remove",
            "--base",
            &base(&ctx, "a", "claude-code"),
        ],
    );
    let denied = run(
        &ctx,
        "mcp:apply",
        &[
            "--only", "mcp:b", "--agent", "codex", "--action", "add", "--spec", spec,
        ],
    );
    assert!(
        !denied.ok,
        "removed accounts' credential variables stay reserved"
    );
    assert!(!contains(&db_bytes(&ctx), "YOUR_FAKE_CLAIMED_A_VALUE"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn failed_mcp_audit_rolls_back_written_config_and_creates_no_phantom_credential_owner() {
    let (root, ctx) = temp_ctx("mcp-claim-audit-rollback");
    let source = configure_secrets_file(&ctx);
    std::fs::write(&source, "YOUR_UNCLAIMED_KEY=YOUR_FAKE_UNCLAIMED_VALUE\n").unwrap();
    write_claude(&ctx, json!({"mcpServers":{}}));
    write_codex(&ctx, "# B stays intact\n");
    let conn = store::open_write(&ctx).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_mcp_audit BEFORE INSERT ON operation_logs BEGIN SELECT RAISE(ABORT, 'injected mcp audit failure'); END;").unwrap();
    drop(conn);
    let before_a = read(&claude_path(&ctx));
    let before_b = read(&codex_path(&ctx));
    let before_source = read(&source);
    let before_db = db_bytes(&ctx);
    let spec = r#"{"command":"mcp","env":{"TOKEN":"${YOUR_UNCLAIMED_KEY}"}}"#;
    let failed = run(
        &ctx,
        "mcp:apply",
        &[
            "--only",
            "mcp:a",
            "--agent",
            "claude-code",
            "--action",
            "add",
            "--spec",
            spec,
        ],
    );
    assert!(!failed.ok);
    assert_eq!(read(&claude_path(&ctx)), before_a);
    assert_eq!(read(&codex_path(&ctx)), before_b);
    assert_eq!(read(&source), before_source);
    assert_eq!(db_bytes(&ctx), before_db);
    let conn = store::open_write(&ctx).unwrap();
    conn.execute_batch("DROP TRIGGER reject_mcp_audit;")
        .unwrap();
    drop(conn);
    // B may claim the variable now because the failed A transaction left no owner.
    apply_ok(
        &ctx,
        &[
            "--only", "mcp:b", "--agent", "codex", "--action", "add", "--spec", spec,
        ],
    );
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let owner: String = conn
        .query_row(
            "SELECT account FROM secret_variable_owners WHERE name='YOUR_UNCLAIMED_KEY'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(owner, "codex");
    drop(conn);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn remove_unknown_entry_cannot_drop_unconfirmed_credential_ownership() {
    let (root, ctx) = temp_ctx("remove-unknown-owner");
    let original =
        "[mcp_servers.legacy]\ncommand = 17\nargs = [\"--token=${YOUR_OLD_ACCOUNT_KEY}\"]\n";
    write_codex(&ctx, original);
    let digest = base(&ctx, "legacy", "codex");
    let before_db = db_bytes(&ctx);
    let args = [
        "--only",
        "mcp:legacy",
        "--agent",
        "codex",
        "--action",
        "remove",
        "--base",
        &digest,
    ];
    for command in ["mcp:preview", "mcp:apply"] {
        let env = run(&ctx, command, &args);
        assert!(!env.ok, "未知条目不能删除尚未确认的凭据归属");
        assert!(
            errors(&env).contains("无法确认凭据归属"),
            "{}",
            errors(&env)
        );
        assert_eq!(read(&codex_path(&ctx)), original);
        assert_eq!(db_bytes(&ctx), before_db);
    }
    let _ = std::fs::remove_dir_all(root);
}

/// 删除：表头形式按既有文本规则（保留上方用户注释、连带删紧邻的旧版注释）；
#[test]
fn account_copy_refuses_credentials_references_and_uncertain_fields_without_writes() {
    let (root, ctx) = temp_ctx("account-copy");
    let unsafe_entries = json!({
        "literal": {"command":"mcp", "env":{"API_KEY":"YOUR_FAKE_KEY"}},
        "reference": {"command":"mcp", "env":{"REF":"${YOUR_FAKE_VAR}"}},
        "header": {"url":"https://mcp.example.test/mcp", "headers":{"Authorization":"YOUR_FAKE_HEADER"}},
        "headerref": {"url":"https://mcp.example.test/mcp", "headers":{"X-Ref":"${YOUR_FAKE_VAR}"}},
        "argument": {"command":"mcp", "args":["--api-key", "YOUR_FAKE_ARG"]},
        "commandref": {"command":"${YOUR_FAKE_VAR}"},
        "commandenvref": {"command":"{env:YOUR_FAKE_VAR}"},
        "urlref": {"url":"https://mcp.example.test/${YOUR_FAKE_VAR}"},
        "urlquery": {"url":"https://mcp.example.test/mcp?opaque=YOUR_FAKE_QUERY"},
        "urluser": {"url":"https://YOUR_FAKE_USER:YOUR_FAKE_PASSWORD@mcp.example.test/mcp"},
        "bearer": {"url":"https://mcp.example.test/mcp", "bearer_token_env_var":"YOUR_FAKE_VAR"},
        "helper": {"command":"mcp", "headersHelper":"YOUR_FAKE_HELPER"},
        "oauth": {"url":"https://mcp.example.test/mcp", "auth":{"type":"oauth"}},
        "unknown": {"command":"mcp", "unrecognizedConfig":"YOUR_FAKE_UNKNOWN"}
    });
    write_claude(&ctx, json!({"mcpServers":unsafe_entries}));
    write_codex(&ctx, "# target stays intact\n");
    let source_before = read(&claude_path(&ctx));
    let target_before = read(&codex_path(&ctx));
    let db_before = db_bytes(&ctx);
    for name in unsafe_entries.as_object().unwrap().keys() {
        let data = status(&ctx);
        let observed = &element(&data, name)["observed"]["agents"]["claude-code"];
        assert_eq!(observed["copy_safe"], false, "{name}");
        assert!(
            !observed["actions"]
                .as_array()
                .unwrap()
                .contains(&json!("copy")),
            "{name}"
        );
        let digest = base(&ctx, name, "claude-code");
        let only = format!("mcp:{name}");
        let args = [
            "--only",
            &only,
            "--agent",
            "claude-code",
            "--to",
            "codex",
            "--action",
            "copy",
            "--base",
            &digest,
        ];
        let preview = run(&ctx, "mcp:preview", &args);
        assert!(!preview.ok, "{name}");
        assert!(
            errors(&preview).contains("不能跨账号复制"),
            "{}",
            errors(&preview)
        );
        let outcome = apply(&ctx, &args);
        assert!(!outcome.ok, "{name}");
        assert!(!serde_json::to_string(&outcome)
            .unwrap()
            .contains("YOUR_FAKE_"));
        assert_eq!(read(&claude_path(&ctx)), source_before);
        assert_eq!(read(&codex_path(&ctx)), target_before);
        assert_eq!(db_bytes(&ctx), db_before);
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn oversized_mcp_account_config_is_blocked_before_read_or_write() {
    let (root, ctx) = temp_ctx("oversized-account-config");
    write_claude(&ctx, json!({"mcpServers": {}}));
    let content = format!(
        "{{\"padding\":\"{}\",\"mcpServers\":{{}}}}",
        "x".repeat(1024 * 1024)
    );
    std::fs::write(claude_path(&ctx), &content).unwrap();
    let before_db = db_bytes(&ctx);
    let observed = status(&ctx);
    let target = observed["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|target| target["agent"] == "claude-code")
        .unwrap();
    assert!(target["error"].as_str().unwrap().contains("1 MiB"));
    let outcome = apply(
        &ctx,
        &[
            "--only",
            "mcp:new",
            "--agent",
            "claude-code",
            "--action",
            "add",
            "--spec",
            r#"{"command":"mcp"}"#,
        ],
    );
    assert!(!outcome.ok);
    assert_eq!(read(&claude_path(&ctx)), content);
    assert_eq!(db_bytes(&ctx), before_db);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mcp_add_edit_enable_and_check_cannot_borrow_another_accounts_variable() {
    let (root, ctx) = temp_ctx("mcp-variable-owner");
    write_claude(
        &ctx,
        json!({"mcpServers":{"owned":{"command":"mcp","env":{"KEY":"${YOUR_MCP_ACCOUNT_A}"}}}}),
    );
    write_codex(
        &ctx,
        "# B stays intact\n[mcp_servers.safe]\ncommand = \"mcp\"\n",
    );
    configure_secrets_file(&ctx);
    let configured = run(
        &ctx,
        "secrets:set",
        &[
            "--var",
            "YOUR_MCP_ACCOUNT_A",
            "--value",
            "YOUR_FAKE_MCP_ACCOUNT_A_VALUE",
        ],
    );
    assert!(configured.ok, "{}", errors(&configured));
    let original_a = read(&claude_path(&ctx));
    let original_b = read(&codex_path(&ctx));
    let original_db = db_bytes(&ctx);
    let spec = r#"{"command":"mcp","env":{"KEY":"${YOUR_MCP_ACCOUNT_A}"}}"#;
    let digest = base(&ctx, "safe", "codex");
    for args in [
        vec![
            "--only", "mcp:new", "--agent", "codex", "--action", "add", "--spec", spec,
        ],
        vec![
            "--only", "mcp:safe", "--agent", "codex", "--action", "edit", "--base", &digest,
            "--spec", spec,
        ],
    ] {
        let preview = run(&ctx, "mcp:preview", &args);
        assert!(!preview.ok);
        assert!(errors(&preview).contains("另一个账号"));
        let outcome = apply(&ctx, &args);
        assert!(!outcome.ok);
        assert_eq!(read(&claude_path(&ctx)), original_a);
        assert_eq!(read(&codex_path(&ctx)), original_b);
        assert_eq!(db_bytes(&ctx), original_db);
    }
    // Embedded argument references are also owned; literal defaults do not weaken source failure.
    for embedded in [
        r#"{"command":"mcp","args":["--token=${YOUR_MCP_ACCOUNT_A}"]}"#,
        r#"{"command":"mcp","args":["${YOUR_MCP_ACCOUNT_A:-fallback}"]}"#,
        r#"{"url":"https://mcp.example.test/${YOUR_MCP_ACCOUNT_A}","headers":{}}"#,
    ] {
        let args = [
            "--only",
            "mcp:embedded",
            "--agent",
            "codex",
            "--action",
            "add",
            "--spec",
            embedded,
        ];
        for command in ["mcp:preview", "mcp:apply"] {
            let result = run(&ctx, command, &args);
            assert!(!result.ok, "embedded refs cannot borrow another account");
            assert_eq!(read(&claude_path(&ctx)), original_a);
            assert_eq!(read(&codex_path(&ctx)), original_b);
            assert_eq!(db_bytes(&ctx), original_db);
        }
    }
    // A's own variable remains a valid input for A.
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:own-new",
            "--agent",
            "claude-code",
            "--action",
            "add",
            "--spec",
            spec,
        ],
    );
    write_codex(&ctx, "# external unsafe B\n[mcp_servers.unsafe]\ncommand = \"mcp\"\nenabled = false\n[mcp_servers.unsafe.env]\nKEY = \"${YOUR_MCP_ACCOUNT_A}\"\n");
    let injected_b = read(&codex_path(&ctx));
    let before_enable_db = db_bytes(&ctx);
    let digest = base(&ctx, "unsafe", "codex");
    let enabled = apply(
        &ctx,
        &[
            "--only",
            "mcp:unsafe",
            "--agent",
            "codex",
            "--action",
            "enable",
            "--base",
            &digest,
        ],
    );
    assert!(!enabled.ok);
    let checked = run(
        &ctx,
        "mcp:check",
        &["--only", "mcp:unsafe", "--agent", "codex"],
    );
    assert!(!checked.ok);
    assert_eq!(read(&codex_path(&ctx)), injected_b);
    assert_eq!(db_bytes(&ctx), before_enable_db);
    let _ = std::fs::remove_dir_all(root);
}

/// 删除：表头形式按既有文本规则（保留上方用户注释、连带删紧邻的旧版注释）；
/// 行内写法用结构化删除；其他表逐字节保留。
#[test]
fn remove_keeps_other_tables_and_comments() {
    let (root, ctx) = temp_ctx("remove");
    let text = "# top\nmodel = \"o3\"\n\n# user note\n# [agent-manager:managed]\n[mcp_servers.target]\ncommand = \"x\"\n[mcp_servers.target.env]\nK = \"v\"\n\n# before keep\n[mcp_servers.keep]\ncommand = \"k\"\n";
    write_codex(&ctx, text);
    let args = [
        "--only",
        "mcp:target",
        "--agent",
        "codex",
        "--action",
        "remove",
        "--base",
        &base(&ctx, "target", "codex"),
    ];
    let before_db = db_bytes(&ctx);
    let preview = run(&ctx, "mcp:preview", &args);
    assert!(preview.ok, "{}", errors(&preview));
    let expected = preview.data["files"][0]["content"].as_str().unwrap();
    assert_eq!(read(&codex_path(&ctx)), text, "预览不得修改目标文件");
    assert_eq!(db_bytes(&ctx), before_db, "预览不得写库");
    apply_ok(&ctx, &args);
    assert_eq!(
        read(&codex_path(&ctx)),
        expected,
        "删除预览须与写入逐字节一致"
    );
    assert_eq!(
        read(&codex_path(&ctx)),
        "# top\nmodel = \"o3\"\n\n# user note\n\n# before keep\n[mcp_servers.keep]\ncommand = \"k\"\n"
    );
    write_codex(
        &ctx,
        "[mcp_servers]\na = { command = \"a\" }\nb = { command = \"b\" }\n",
    );
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:a",
            "--agent",
            "codex",
            "--action",
            "remove",
            "--base",
            &base(&ctx, "a", "codex"),
        ],
    );
    assert_eq!(
        read(&codex_path(&ctx)),
        "[mcp_servers]\nb = { command = \"b\" }\n"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn toml_remove_drops_adjacent_legacy_comment_and_keeps_others() {
    use agent_manager_engine::native_mcp::targets::toml_remove_server_text;
    let input = "# top user comment\nmodel = \"o3\"\n\n# user note before target\n# [agent-manager:managed]\n[mcp_servers.target]\ncommand = \"x\"\n# inside target\n[mcp_servers.target.env]\nK = \"v\"\n\n# [agent-manager:managed]\n[mcp_servers.keep]\ncommand = \"k\"\n# user comment before profiles\n[profiles.p]\nk = 1\n";
    let expected = "# top user comment\nmodel = \"o3\"\n\n# user note before target\n\n# [agent-manager:managed]\n[mcp_servers.keep]\ncommand = \"k\"\n# user comment before profiles\n[profiles.p]\nk = 1\n";
    assert_eq!(toml_remove_server_text(input, "target"), expected);
    let untouched = "# [agent-manager:managed]\n[mcp_servers.other]\ncommand = \"o\"";
    assert_eq!(toml_remove_server_text(untouched, "target"), untouched);
    let user_comment = "# mine\n[mcp_servers.target]\ncommand = \"x\"\n";
    assert_eq!(toml_remove_server_text(user_comment, "target"), "# mine\n");
}

/// 旧版标记：读取时剥离、status 提示；编辑时顺带清除条目级/env 内标记与表头旧注释，
/// 其他注释保留；不做读取时的自动改写。
#[test]
fn legacy_markers_are_reported_and_cleared_on_edit() {
    let (root, ctx) = temp_ctx("legacy-marker");
    let text = "# mine\n# [agent-manager:managed]\n[mcp_servers.old]\ncommand = \"x\"\n_MANAGED_BY = \"agent-manager\"\n[mcp_servers.old.env]\n_MANAGED_BY = \"agent-manager\"\nK = \"v\"\n";
    write_codex(&ctx, text);
    let c = cell(&ctx, "old", "codex");
    assert_eq!(c["legacy_marker"], true);
    assert!(c["notice"].as_str().unwrap().contains("旧版"));
    assert_eq!(
        c["entry"]["env"],
        json!({"K": "v"}),
        "标记不进入 env；不像凭据的字面量照原值显示"
    );
    assert_eq!(read(&codex_path(&ctx)), text, "读取不改写");
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:old",
            "--agent",
            "codex",
            "--action",
            "edit",
            "--base",
            c["base"].as_str().unwrap(),
            "--spec",
            r#"{"command":"x","env":{"K":"<literal>"}}"#,
        ],
    );
    let after = read(&codex_path(&ctx));
    assert!(
        !after.contains("_MANAGED_BY") && !after.contains("agent-manager"),
        "{after}"
    );
    assert!(after.starts_with("# mine\n[mcp_servers.old]\n"), "{after}");
    assert_eq!(
        codex_doc(&ctx)["mcp_servers"]["old"]["env"]["K"].as_str(),
        Some("v")
    );
    assert_eq!(cell(&ctx, "old", "codex")["legacy_marker"], false);
    let _ = std::fs::remove_dir_all(root);
}

/// Codex 连续新增/编辑不累积空行：新表与前文恰好隔一个空行，重复编辑字节稳定。
#[test]
fn codex_repeated_writes_do_not_accumulate_blank_lines() {
    let (root, ctx) = temp_ctx("blank-lines");
    let user = "# user\n[mcp_servers.userx]\ncommand = \"userx\"\n";
    write_codex(&ctx, user);
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:s1",
            "--agent",
            "codex",
            "--action",
            "add",
            "--spec",
            r#"{"command":"npx","args":["-y","p"]}"#,
        ],
    );
    let text = read(&codex_path(&ctx));
    assert!(
        text.starts_with(&format!("{user}\n[mcp_servers.s1]\n")),
        "{text:?}"
    );
    for i in 0..3 {
        apply_ok(
            &ctx,
            &[
                "--only",
                "mcp:s1",
                "--agent",
                "codex",
                "--action",
                "edit",
                "--base",
                &base(&ctx, "s1", "codex"),
                "--spec",
                &format!(r#"{{"command":"npx","args":["-y","p{}"]}}"#, i % 2),
            ],
        );
        let now = read(&codex_path(&ctx));
        assert!(!now.contains("\n\n\n"), "{now:?}");
        assert!(now.starts_with(user));
    }
    let _ = std::fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 旧库迁移
// ---------------------------------------------------------------------------

fn legacy_version(def: &Value) -> String {
    let mut clean = def.as_object().unwrap().clone();
    clean.retain(|k, _| !k.starts_with('_') && k != "enabled" && k != "agents");
    digest_of(&Value::Object(clean))
}

/// 用旧版表结构造一个旧库：服务库定义 + mcp_bindings。
fn seed_legacy(ctx: &Context, defs: Value, bindings: &[(&str, &str, &Value, bool)]) {
    let conn = store::open_write(ctx).unwrap();
    let mut policy = store::policy_from_connection(&conn).unwrap();
    policy["mcp"] = json!({"definitions": defs});
    store::write_policy(&conn, &policy).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS mcp_bindings (
            server TEXT NOT NULL, agent TEXT NOT NULL, version TEXT NOT NULL,
            definition_json TEXT NOT NULL, enabled INTEGER NOT NULL, assigned INTEGER NOT NULL,
            applied_digest TEXT NOT NULL, updated_at TEXT NOT NULL, PRIMARY KEY (server, agent))",
    )
    .unwrap();
    for (server, agent, def, enabled) in bindings {
        conn.execute(
            "INSERT INTO mcp_bindings VALUES (?1, ?2, ?3, ?4, ?5, 1, '', '0')",
            rusqlite::params![
                server,
                agent,
                legacy_version(def),
                def.to_string(),
                *enabled as i64
            ],
        )
        .unwrap();
    }
}

fn table_exists(ctx: &Context, name: &str) -> bool {
    let conn = store::open_readonly(ctx).unwrap().unwrap();
    conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
        > 0
}

/// 必测 6：旧库三类（已分配且存在 / 已分配但停用或缺失 / 从未分配）+「库里定义比已应用的新」。
/// 读路径即时推导与写路径落库结果一致；迁移不改实际文件；旧表与旧定义被清掉。
#[test]
fn legacy_library_is_migrated_on_write_without_touching_files() {
    let (root, ctx) = temp_ctx("legacy");
    let source = configure_secrets_file(&ctx);
    std::fs::write(&source, "AM_T9_TOKEN=resolved-t9\n").unwrap();
    let present =
        json!({"command": "npx", "args": ["-y", "present"], "env": {"TOKEN": "${AM_T9_TOKEN}"}});
    let disabled = json!({"command": "npx", "args": ["-y", "disabled"], "env": {"TOKEN": "${AM_T9_TOKEN}"}, "startup_timeout_sec": 20});
    let never = json!({"command": "npx", "args": ["-y", "never"], "agents": ["codex"]});
    let newer_old = json!({"command": "npx", "args": ["-y", "newer@1"]});
    let newer_new = json!({"command": "npx", "args": ["-y", "newer@2"]});
    seed_legacy(
        &ctx,
        json!({"present": present, "disabled": disabled, "never": never, "newer": newer_new,
               "to-forget": {"command": "x"}}),
        &[
            ("present", "claude-code", &present, true),
            ("disabled", "codex", &disabled, false),
            ("newer", "claude-code", &newer_old, true),
        ],
    );
    write_claude(
        &ctx,
        json!({"mcpServers": {
            "present": {"command": "npx", "args": ["-y", "present"], "env": {"TOKEN": "resolved-t9"}},
            "newer": {"command": "npx", "args": ["-y", "newer@1"], "env": {}}
        }}),
    );
    write_codex(&ctx, "# 用户\n[mcp_servers.user]\ncommand = \"u\"\n");
    let claude_before = read(&claude_path(&ctx));
    let codex_before = read(&codex_path(&ctx));

    let before = status(&ctx);
    let view = |data: &Value| -> Vec<Value> {
        ["present", "disabled", "never", "newer"]
            .iter()
            .map(|n| {
                let e = element(data, n);
                json!({"spec": e["spec"], "state": e["state"], "agents": e["observed"]["agents"]})
            })
            .collect()
    };
    let b = view(&before);
    assert_eq!(b[0]["agents"]["claude-code"]["source"], "file");
    assert_eq!(
        b[0]["spec"]["saved"],
        Value::Null,
        "已应用且未变的定义不另存"
    );
    assert_eq!(b[1]["agents"]["codex"]["source"], "saved");
    assert_eq!(
        b[1]["agents"]["codex"]["entry"]["env"]["TOKEN"],
        "${AM_T9_TOKEN}"
    );
    assert_eq!(b[1]["agents"]["codex"]["entry"]["startup_timeout_sec"], 20);
    assert_eq!(b[2]["spec"]["saved"]["definition"]["command"], "npx");
    assert!(b[2]["spec"]["saved"]["definition"].get("agents").is_none());
    assert_eq!(b[3]["agents"]["claude-code"]["source"], "file");
    assert_eq!(
        b[3]["spec"]["saved"]["definition"]["args"],
        json!(["-y", "newer@2"]),
        "较新的定义不丢"
    );
    assert!(table_exists(&ctx, "mcp_bindings"), "读路径不迁移");

    // 任一 MCP 写动作触发迁移：这里丢掉一个无关的已保存定义
    apply_ok(&ctx, &["--only", "mcp:to-forget", "--action", "forget"]);
    assert!(!table_exists(&ctx, "mcp_bindings"), "旧分配表已清掉");
    let policy = store::effective_policy(&ctx).unwrap();
    assert_eq!(policy["mcp"], json!({"definitions": {}}));
    assert_eq!(view(&status(&ctx)), b, "迁移前后视图一致");
    assert_eq!(read(&claude_path(&ctx)), claude_before, "迁移不改实际文件");
    assert_eq!(read(&codex_path(&ctx)), codex_before, "迁移不改实际文件");
    assert!(op_details(&ctx).iter().any(|(op, _)| op == "mcp:migrate"));

    // 迁移来的 Codex 快照可以启用：env 引用写入时解析
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:disabled",
            "--agent",
            "codex",
            "--action",
            "enable",
            "--base",
            &base(&ctx, "disabled", "codex"),
        ],
    );
    let doc = codex_doc(&ctx);
    assert_eq!(
        doc["mcp_servers"]["disabled"]["env"]["TOKEN"].as_str(),
        Some("resolved-t9")
    );
    assert_eq!(
        doc["mcp_servers"]["disabled"]["startup_timeout_sec"].as_integer(),
        Some(20)
    );
    let _ = std::fs::remove_dir_all(root);
}

/// 裁决第 4 条：policy:set 写入的 MCP 定义保留校验，并落成「未分配的已保存定义」。
#[test]
fn policy_set_mcp_definition_becomes_saved_definition() {
    let (root, ctx) = temp_ctx("policy-set");
    let bad = run(
        &ctx,
        "policy:set",
        &[
            "--layer",
            "local",
            "--key",
            "mcp.definitions.bad",
            "--value",
            r#"{"command":"x","env":{"K":"literal"}}"#,
        ],
    );
    assert!(!bad.ok, "校验保留：明文 env 仍被拒绝");
    let ok = run(
        &ctx,
        "policy:set",
        &[
            "--layer",
            "local",
            "--key",
            "mcp.definitions.demo",
            "--value",
            r#"{"command":"npx","args":["-y","demo"],"env":{"K":"${AM_T19}"},"_delete":false,"_replace":true}"#,
        ],
    );
    assert!(ok.ok, "{}", errors(&ok));
    assert_eq!(
        store::effective_policy(&ctx).unwrap()["mcp"],
        json!({"definitions": {}})
    );
    let data = status(&ctx);
    let demo = element(&data, "demo");
    assert_eq!(demo["spec"]["saved"]["definition"]["env"]["K"], "${AM_T19}");
    assert_eq!(demo["state"], "disabled");
    assert_eq!(demo["actions"][0]["id"], "forget");
    apply_ok(&ctx, &["--only", "mcp:demo", "--action", "forget"]);
    assert!(status(&ctx)["elements"].as_array().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn imported_snapshot_case_aliases_are_rejected_before_database_creation() {
    let (root, ctx) = temp_ctx("backup-case-alias");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let policy = store::effective_policy(&ctx).unwrap();
    let row = |agent: &str, name: &str| {
        json!({
            "server": "remote", "agent": agent,
            "entry": {"command": "mcp", "env": {"KEY": format!("${{{name}}}")}},
            "resolve": {"env": [], "headers": []}, "origin": "disabled"
        })
    };
    for (claude_name, codex_name) in [
        ("your_case_alias_key", "YOUR_CASE_ALIAS_KEY"),
        ("YOUR_CASE_ALIAS_KEY", "your_case_alias_key"),
    ] {
        let payload = json!({
            "format": "agent-manager/config-backup/v1", "policy": policy,
            "sources": {}, "mcp_saved": [
                row("claude-code", claude_name), row("codex", codex_name)
            ]
        })
        .to_string();
        let rejected = run(&ctx, "config:import", &["--value", &payload]);
        assert!(!rejected.ok, "大小写别名不能给两个账号共享同一变量");
        assert!(
            errors(&rejected).contains("拒绝导入跨账号 MCP 引用"),
            "{}",
            errors(&rejected)
        );
        assert!(!ctx.data_dir.exists(), "必须在建库前拒绝候选归属冲突");
        assert!(!claude_path(&ctx).exists());
        assert!(!codex_path(&ctx).exists());
        assert_eq!(store::effective_policy(&ctx).unwrap(), policy);
    }
    let _ = std::fs::remove_dir_all(root);
}

/// 裁决第 8 条：配置备份包含已保存项（快照只含引用与普通字面量），换机导入后可启用。
#[test]
fn saved_items_roundtrip_through_config_backup() {
    let (root, ctx) = temp_ctx("backup-src");
    write_claude(
        &ctx,
        json!({"mcpServers": {"fs": {"command": "npx", "args": ["-y", "fs"],
            "env": {"AM_T22_KEY": "${AM_T22_KEY}", "NODE_ENV": "production"}}}}),
    );
    configure_secrets_file(&ctx);
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:fs",
            "--agent",
            "claude-code",
            "--action",
            "disable",
            "--base",
            &base(&ctx, "fs", "claude-code"),
        ],
    );
    let exported = run(&ctx, "config:export", &[]);
    assert!(exported.ok, "{}", errors(&exported));
    let payload = serde_json::to_string(&exported.data).unwrap();
    assert!(!payload.contains("sk-ant-api03-backup"), "备份不含明文");
    let saved = exported.data["mcp_saved"].as_array().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0]["entry"]["env"]["AM_T22_KEY"], "${AM_T22_KEY}");

    let (root2, ctx2) = temp_ctx("backup-dst");
    std::fs::create_dir_all(&ctx2.home_dir).unwrap();
    configure_secrets_file(&ctx2);
    let imported = run(&ctx2, "config:import", &["--value", &payload]);
    assert!(imported.ok, "{}", errors(&imported));
    let again = run(&ctx2, "config:export", &[]);
    assert_eq!(
        again.data["mcp_saved"], exported.data["mcp_saved"],
        "往返一致"
    );
    // Existing references remain references after a round trip; no credential migration.
    apply_ok(
        &ctx2,
        &[
            "--only",
            "mcp:fs",
            "--agent",
            "claude-code",
            "--action",
            "enable",
            "--base",
            &base(&ctx2, "fs", "claude-code"),
        ],
    );
    assert_eq!(
        claude_entry(&ctx2, "fs")["env"],
        json!({"AM_T22_KEY": "${AM_T22_KEY}", "NODE_ENV": "production"})
    );
    // 带明文的已保存项拒绝导入
    let bad = payload.replace("${AM_T22_KEY}", "sk-ant-api03-injected0123456789abcdef");
    let (root3, ctx3) = temp_ctx("backup-bad");
    let rejected = run(&ctx3, "config:import", &["--value", &bad]);
    assert!(!rejected.ok, "含疑似凭据的已保存项必须拒绝");
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(root2);
    let _ = std::fs::remove_dir_all(root3);
}

// ---------------------------------------------------------------------------
// 参数、锁与审计
// ---------------------------------------------------------------------------

/// 严格参数：退役动作/参数、缺 base、坏摘要、组合错误一律在建库前拒绝。
#[test]
fn strict_args_reject_retired_and_malformed_requests_before_any_write() {
    let (root, ctx) = temp_ctx("strict-args");
    let d = "a".repeat(64);
    for (args, expect) in [
        (
            vec![
                "--only",
                "mcp:x",
                "--agent",
                "codex",
                "--action",
                "add",
                "--spec",
                "{}",
                "--dry-run",
            ],
            "--dry-run",
        ),
        (
            vec![
                "--only",
                "mcp:x",
                "--agent",
                "codex",
                "--action",
                "add",
                "--allow-takeover",
            ],
            "--allow-takeover",
        ),
        (
            vec!["--only", "mcp:x", "--agent", "codex", "--action", "update"],
            "不再支持",
        ),
        (vec!["--only", "mcp:x", "--action", "upgrade"], "不再支持"),
        (
            vec![
                "--only", "mcp:x", "--agent", "codex", "--action", "edit", "--spec", "{}",
            ],
            "--base",
        ),
        (
            vec![
                "--only", "mcp:x", "--agent", "codex", "--action", "disable", "--base", "ABC",
            ],
            "十六进制",
        ),
        (
            vec!["--only", "mcp:x", "--agent", "codex", "--action", "forget"],
            "--agent",
        ),
        (
            vec![
                "--only", "mcp:x", "--agent", "codex", "--to", "codex", "--action", "copy",
                "--base", &d,
            ],
            "同一个助手",
        ),
        (
            vec![
                "--only",
                "mcp:x",
                "--agent",
                "codex",
                "--to",
                "claude-code",
                "--action",
                "copy",
                "--base",
                &d,
                "--replace",
            ],
            "--to-base",
        ),
        (
            vec![
                "--only",
                "mcp:bad.name",
                "--agent",
                "codex",
                "--action",
                "remove",
                "--base",
                &d,
            ],
            "非法 server",
        ),
        (
            vec![
                "--only", "mcp:x", "--agent", "codex", "--action", "remove", "--base", &d, "stray",
            ],
            "位置参数",
        ),
        (
            vec![
                "--only", "mcp:x", "--agent", "codex", "--action", "remove", "--base", &d,
                "--bogus",
            ],
            "未知选项",
        ),
    ] {
        let err = apply_err(&ctx, &args);
        assert!(err.contains(expect), "{args:?}: {err}");
    }
    assert!(!ctx.data_dir.exists(), "参数错误不得建库");
    let bad_status = run(&ctx, "mcp:status", &["--bogus"]);
    assert!(!bad_status.ok);
    let _ = std::fs::remove_dir_all(root);
}

/// 并发持有 apply 锁时立即拒绝；成功写入记审计，审计与日志不含命令参数、地址或取值。
#[test]
fn apply_lock_and_audit_never_record_values() {
    let (root, ctx) = temp_ctx("lock-audit");
    write_claude(&ctx, json!({"mcpServers": {}}));
    std::fs::create_dir_all(&ctx.data_dir).unwrap();
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(ctx.data_dir.join("agent-manager.apply.lock"))
        .unwrap();
    lock_file.lock_exclusive().unwrap();
    let spec = r#"{"command":"secret-cmd-marker","args":["--arg-marker"],"env":{"PLAIN":"plain-env-marker"}}"#;
    let worker_ctx = Context {
        data_dir: ctx.data_dir.clone(),
        home_dir: ctx.home_dir.clone(),
    };
    let spec_owned = spec.to_string();
    let contended = std::thread::spawn(move || {
        execute(
            &worker_ctx,
            &request(
                "mcp:apply",
                &[
                    "--only",
                    "mcp:x",
                    "--agent",
                    "claude-code",
                    "--action",
                    "add",
                    "--spec",
                    &spec_owned,
                ],
            ),
        )
    })
    .join()
    .unwrap();
    assert!(!contended.ok, "持有锁时必须拒绝");
    drop(lock_file);

    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:x",
            "--agent",
            "claude-code",
            "--action",
            "add",
            "--spec",
            spec,
        ],
    );
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:r",
            "--agent",
            "claude-code",
            "--action",
            "add",
            "--spec",
            r#"{"url":"https://url-marker.example.com/mcp?token=query-marker"}"#,
        ],
    );
    let ops = op_details(&ctx);
    let add = ops
        .iter()
        .find(|(op, _)| op == "mcp:add")
        .expect("要有审计记录");
    assert!(add.1.contains("\"action\":\"add\""), "{}", add.1);
    let bytes = db_bytes(&ctx);
    for marker in [
        "secret-cmd-marker",
        "--arg-marker",
        "plain-env-marker",
        "url-marker",
        "query-marker",
    ] {
        assert!(
            !contains(&bytes, marker),
            "数据库（含审计）不得记录 {marker}"
        );
    }
    let logs = ctx.data_dir.join("logs");
    if logs.exists() {
        for entry in std::fs::read_dir(&logs).unwrap().flatten() {
            let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
            for marker in ["secret-cmd-marker", "plain-env-marker", "query-marker"] {
                assert!(!text.contains(marker), "诊断日志不得记录 {marker}");
            }
        }
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn removed_account_is_not_a_target_and_cannot_receive_copies() {
    let (root, ctx) = temp_ctx("removed-account");
    let definition = json!({
        "display_name": "Gone",
        "bin": "claude",
        "skills_dir": "~/.claude-gone/skills",
        "mcp_config": {"path": "~/.claude-gone/.claude.json", "format": "json"},
        "profile": {"base_agent": "claude-code", "kind": "claude", "config_dir": "~/.claude-gone"},
        "_delete": false,
        "_replace": true,
    })
    .to_string();
    let set = run(
        &ctx,
        "policy:set",
        &[
            "--layer",
            "local",
            "--key",
            "agents.definitions.gone",
            "--value",
            &definition,
        ],
    );
    assert!(set.ok, "{}", errors(&set));
    let listed = status(&ctx);
    assert!(listed["targets"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["agent"] == "gone"));

    // 移除账号只留删除标记（目录保留）
    let removed = run(
        &ctx,
        "policy:set",
        &[
            "--layer",
            "local",
            "--key",
            "agents.definitions.gone",
            "--value",
            r#"{"_delete": true}"#,
        ],
    );
    assert!(removed.ok, "{}", errors(&removed));
    let after = status(&ctx);
    assert!(
        after["targets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["agent"] != "gone"),
        "已移除的账号不能再列为 MCP 目标"
    );

    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:fs",
            "--agent",
            "claude-code",
            "--action",
            "add",
            "--spec",
            r#"{"command":"npx","args":["-y","fs"]}"#,
        ],
    );
    let base = base(&ctx, "fs", "claude-code");
    apply_err(
        &ctx,
        &[
            "--only",
            "mcp:fs",
            "--agent",
            "claude-code",
            "--to",
            "gone",
            "--action",
            "copy",
            "--base",
            &base,
        ],
    );
    assert!(
        !ctx.home_dir.join(".claude-gone/.claude.json").exists(),
        "不能写进已移除账号的目录"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// 回归（2026-09-30 真机预览发现）：往 ~/.claude.json 写一个 MCP 服务时，文件里无关的浮点数被改写
/// （`13.220812500000001 → 13.2208125`、`1.1291944000000003 → 1.1291944000000005`）。
/// 读入再写回必须与 Claude Code（Node JSON.stringify）写出的数字逐字一致。
#[test]
fn json_rewrite_keeps_unrelated_floats_byte_identical() {
    let text = "[13.220812500000001,0.0013390000000000001,3.6376911999999995,1.1291944000000003,1.1282134000000001,0.1,1e-7,123456789012345680]";
    let value: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(serde_json::to_string(&value).unwrap(), text);
}

/// 「（保留原值）」看不出原值：不像凭据的 env 字面量照原值显示（保存时原样写回）；像凭据的仍不交明文，
/// 只给头尾提示；远程请求头的字面量（写入只接受引用或保留原值）给提示：普通值给完整值，像凭据的给头尾。
#[test]
fn literal_values_show_plain_or_hint_without_leaking_secrets() {
    let (root, ctx) = temp_ctx("literal-hints");
    let secret = "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789";
    write_claude(
        &ctx,
        json!({"mcpServers": {
            "s": {"command": "npx", "args": ["-y", "x"], "env": {"NODE_ENV": "production", "OPENAI_API_KEY": secret, "SHORT_TOKEN": "abc123", "REF": "${REF}"}},
            "r": {"type": "http", "url": "https://mcp.example.com/mcp", "headers": {"X-Region": "cn-east", "Authorization": format!("Bearer {secret}")}}
        }}),
    );
    let c = cell(&ctx, "s", "claude-code");
    assert_eq!(
        c["entry"]["env"],
        json!({"NODE_ENV": "production", "OPENAI_API_KEY": "<literal>", "SHORT_TOKEN": "<literal>", "REF": "${REF}"})
    );
    assert_eq!(
        c["literal_hints"]["env"],
        json!({"OPENAI_API_KEY": "sk-…6789", "SHORT_TOKEN": "已隐藏，6 位"})
    );
    let r = cell(&ctx, "r", "claude-code");
    assert_eq!(
        r["entry"]["headers"],
        json!({"X-Region": "<literal>", "Authorization": "<literal>"})
    );
    assert_eq!(
        r["literal_hints"]["headers"],
        json!({"X-Region": "cn-east", "Authorization": "Bea…6789"})
    );
    let all = serde_json::to_string(&status(&ctx)).unwrap();
    assert!(!all.contains("abcdefghijklmnop"), "明文密钥不进状态");
    assert!(!all.contains("abc123"), "短密钥也不进状态");
    let _ = std::fs::remove_dir_all(root);
}

/// 不误伤原始配置：原条目没有的字段（env、startup_timeout_sec、type 等），编辑时即使界面发来空的 env
/// 也不能凭空加上；只改用户改了的那一项，条目里其余字段与文件其余部分逐字不变。
#[test]
fn edit_never_adds_fields_the_original_entry_did_not_have() {
    let (root, ctx) = temp_ctx("no-added-fields");
    write_claude(
        &ctx,
        json!({"numStartups": 3, "mcpServers": {"bare": {"command": "npx", "args": ["-y", "bare-mcp"]}}}),
    );
    write_codex(&ctx, "# 我的注释\nmodel = \"gpt-5\"\n\n[mcp_servers.bare]\ncommand = \"npx\"\nargs = [\"-y\", \"bare-mcp\"]\n");
    for agent in ["claude-code", "codex"] {
        let b = base(&ctx, "bare", agent);
        // 界面保存时发出的形态：env 为空对象、args 改了一项
        let spec = r#"{"command":"npx","args":["-y","bare-mcp@2"],"env":{}}"#;
        apply_ok(
            &ctx,
            &[
                "--only", "mcp:bare", "--agent", agent, "--action", "edit", "--base", &b, "--spec",
                spec,
            ],
        );
    }
    let entry = claude_entry(&ctx, "bare");
    assert_eq!(
        entry,
        json!({"command": "npx", "args": ["-y", "bare-mcp@2"]}),
        "Claude 条目不多出字段"
    );
    let doc: serde_json::Value = serde_json::from_str(&read(&claude_path(&ctx))).unwrap();
    assert_eq!(doc["numStartups"], 3);
    let codex = read(&codex_path(&ctx));
    assert!(
        codex.starts_with("# 我的注释\nmodel = \"gpt-5\"\n"),
        "Codex 文件其余部分不动：{codex}"
    );
    assert!(
        !codex.contains("[mcp_servers.bare.env]") && !codex.contains("env ="),
        "Codex 条目不多出 env：{codex}"
    );
    assert!(
        !codex.contains("startup_timeout"),
        "不加原来没有的超时：{codex}"
    );
    assert!(codex.contains("\"bare-mcp@2\""));
    let _ = std::fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// mcp:check：测试连接（只返回结果，不写文件、不写库）
// ---------------------------------------------------------------------------

#[test]
fn clean_markers_previews_one_entry_preserves_fields_and_rejects_stale_input() {
    let (root, ctx) = temp_ctx("clean-markers");
    let original = "# user settings\nmodel = \"example\"\n\n[mcp_servers.old]\n_MANAGED_BY = \"agent-manager\"\ncommand = \"uvx\"\nargs = [\"serve\"]\nenabled = false\ncwd = \"/work\"\nrequired = true\nenabled_tools = [\"search\"]\n[mcp_servers.old.env]\n_MANAGED_BY = \"agent-manager\"\nTOKEN = \"example-value\"\n[mcp_servers.other]\ncommand = \"other\"\n_MANAGED_BY = \"keep-other\"\n";
    write_codex(&ctx, original);
    write_claude(&ctx, json!({"mcpServers":{"old":{"command":"different"}}}));
    let other_account = read(&claude_path(&ctx));
    let before = codex_doc(&ctx);
    let cell = cell(&ctx, "old", "codex");
    assert!(cell["actions"]
        .as_array()
        .unwrap()
        .contains(&json!("clean-markers")));
    let args = [
        "--only",
        "mcp:old",
        "--agent",
        "codex",
        "--action",
        "clean-markers",
        "--base",
        cell["base"].as_str().unwrap(),
    ];
    let preview = run(&ctx, "mcp:preview", &args);
    assert!(preview.ok, "{}", errors(&preview));
    assert_eq!(read(&codex_path(&ctx)), original);
    assert!(!ctx.data_dir.exists(), "preview must not create state");
    let changed = original.replace("required = true", "required = false");
    write_codex(&ctx, &changed);
    assert!(apply_err(&ctx, &args).contains("外部修改"));
    assert_eq!(read(&codex_path(&ctx)), changed);
    assert!(
        !ctx.data_dir.exists(),
        "stale cleanup must not create state"
    );
    write_codex(&ctx, original);
    apply_ok(&ctx, &args);
    assert_eq!(
        read(&codex_path(&ctx)),
        preview.data["files"][0]["content"].as_str().unwrap()
    );
    let mut expected = before;
    let old = expected["mcp_servers"]["old"].as_table_mut().unwrap();
    old.remove("_MANAGED_BY");
    old["env"].as_table_mut().unwrap().remove("_MANAGED_BY");
    assert_eq!(codex_doc(&ctx), expected);
    assert!(read(&codex_path(&ctx)).starts_with("# user settings\n"));
    assert_eq!(read(&claude_path(&ctx)), other_account);
    let _ = std::fs::remove_dir_all(root);
}

/// 假的 stdio MCP 服务：先打一行日志和一条通知，再回 initialize；读到 initialized 与 tools/list 后回工具清单。
#[cfg(unix)]
const FAKE_SERVER: &str = r#"#!/bin/sh
touch "$(dirname "$0")/ran"
read line
echo 'starting fake server'
echo '{"jsonrpc":"2.0","method":"notifications/message","params":{}}'
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"1.2.3"}}}'
read line
read line
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"search"},{"name":"fetch"}]}}'
read line
"#;

#[cfg(unix)]
fn fake_server(ctx: &Context, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = ctx.home_dir.join("servers");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fake-mcp");
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn check_runs_a_stdio_server_lists_tools_and_writes_nothing() {
    let (root, ctx) = temp_ctx("check-ok");
    let server = fake_server(&ctx, FAKE_SERVER);
    write_claude(
        &ctx,
        json!({"mcpServers": {"fake": {"command": server.to_str().unwrap()}}}),
    );
    let before = read(&claude_path(&ctx));
    let env = run(
        &ctx,
        "mcp:check",
        &["--only", "mcp:fake", "--agent", "claude-code"],
    );
    assert!(env.ok, "{}", errors(&env));
    let data = &env.data;
    assert_eq!(data["ok"], true, "{data}");
    assert_eq!(data["stage"], "done");
    assert_eq!(data["server_info"]["name"], "fake");
    assert_eq!(data["server_info"]["version"], "1.2.3");
    assert_eq!(data["tool_count"], 2);
    assert_eq!(data["tools"], json!(["search", "fetch"]));
    assert_eq!(read(&claude_path(&ctx)), before);
    assert!(
        !ctx.data_dir.join("native-manager.db").exists(),
        "测试连接不写库"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn check_reaps_background_descendants_after_the_server_exits() {
    let (root, ctx) = temp_ctx("check-orphan");
    let server = fake_server(
        &ctx,
        r#"#!/bin/sh
(printf ready > "${0}.ready"; sleep 2; printf survived > "${0}.late") &
while [ ! -f "${0}.ready" ]; do sleep 0.01; done
read line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"orphan-fixture","version":"1"}}}'
IFS= read -r initialized || exit 2
exit 0
"#,
    );
    write_claude(
        &ctx,
        json!({"mcpServers":{"fake":{"command":server.to_str().unwrap()}}}),
    );
    let before = read(&claude_path(&ctx));
    let result = run(
        &ctx,
        "mcp:check",
        &["--only", "mcp:fake", "--agent", "claude-code"],
    );
    assert!(result.ok && result.data["ok"] == true, "{result:?}");
    assert!(
        server.with_extension("ready").exists(),
        "descendant must actually start"
    );
    assert_eq!(read(&claude_path(&ctx)), before);
    assert!(!ctx.data_dir.exists());
    std::thread::sleep(std::time::Duration::from_millis(2500));
    let survived = server.with_extension("late").exists();
    let _ = std::fs::remove_dir_all(root);
    assert!(
        !survived,
        "MCP descendant survived after the server exited and the check returned"
    );
}

#[cfg(unix)]
#[test]
fn check_uses_configured_or_selected_cwd_and_rejects_stale_entries_before_start() {
    let (root, ctx) = temp_ctx("check-cwd");
    let server = fake_server(
        &ctx,
        &FAKE_SERVER.replace("touch \"$(dirname \"$0\")/ran\"", "pwd > \"${0}.cwd\""),
    );
    let project = ctx.home_dir.join("project with spaces");
    std::fs::create_dir_all(&project).unwrap();
    for configured in [true, false] {
        write_codex(
            &ctx,
            &format!(
                "[mcp_servers.fake]\ncommand = {:?}\n{}",
                server.to_str().unwrap(),
                if configured {
                    format!("cwd = {:?}\n", project.to_str().unwrap())
                } else {
                    String::new()
                }
            ),
        );
        let base = cell(&ctx, "fake", "codex")["base"]
            .as_str()
            .unwrap()
            .to_string();
        let result = run(
            &ctx,
            "mcp:check",
            &[
                "--only",
                "mcp:fake",
                "--agent",
                "codex",
                "--base",
                &base,
                "--cwd",
                ctx.home_dir.to_str().unwrap(),
            ],
        );
        assert!(result.ok && result.data["ok"] == true, "{result:?}");
        let expected = if configured { &project } else { &ctx.home_dir };
        assert_eq!(
            read(&server.with_extension("cwd")).trim(),
            std::fs::canonicalize(expected).unwrap().to_str().unwrap()
        );
        assert_eq!(result.data["cwd"], expected.to_str().unwrap());
        assert_eq!(
            result.data["cwd_source"],
            if configured { "config" } else { "selected" }
        );
        assert_eq!(result.data["startup_timeout_sec"], 10.0);
        assert_eq!(result.data["tool_timeout_sec"], 60.0);
        assert_eq!(result.data["base"], base);
    }
    std::fs::remove_file(server.with_extension("cwd")).unwrap();
    let stale = run(
        &ctx,
        "mcp:check",
        &[
            "--only",
            "mcp:fake",
            "--agent",
            "codex",
            "--base",
            &"0".repeat(64),
        ],
    );
    assert!(!stale.ok && errors(&stale).contains("配置已变化"));
    let invalid = run(
        &ctx,
        "mcp:check",
        &[
            "--only", "mcp:fake", "--agent", "codex", "--cwd", "relative",
        ],
    );
    assert!(!invalid.ok);
    assert!(!server.with_extension("cwd").exists());
    assert!(!ctx.data_dir.exists());
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn check_does_not_report_success_when_tools_fail_or_initialize_is_invalid() {
    let (root, ctx) = temp_ctx("check-tools");
    for (replacement, expected_stage) in [
        ("printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":2,\"error\":{\"code\":-32603,\"message\":\"tools failed\"}}'", "tools"),
        ("printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}'", "tools"),
        ("exit 7", "tools"),
        ("sleep 3", "tools"),
        ("read line", "tools"),
    ] {
        let body = FAKE_SERVER.lines().map(|line| if line.contains("\"id\":2") { replacement } else { line }).collect::<Vec<_>>().join("\n");
        let server = fake_server(&ctx, &body);
        write_codex(&ctx, &format!("[mcp_servers.fake]\ncommand = {:?}\ntool_timeout_sec = 1\n", server.to_str().unwrap()));
        let result = run(&ctx, "mcp:check", &["--only", "mcp:fake", "--agent", "codex"]);
        assert!(result.ok, "{}", errors(&result));
        assert_eq!(result.data["ok"], false, "{replacement}: {}", result.data);
        assert_eq!(result.data["stage"], expected_stage);
        if replacement == "exit 7" { assert_eq!(result.data["exit_code"], 7); }
        if replacement == "sleep 3" || replacement == "read line" { assert_eq!(result.data["exit_code"], Value::Null, "forced cleanup is not a server exit code"); }
    }
    let invalid = FAKE_SERVER.replace("\"protocolVersion\":\"2025-06-18\",", "");
    fake_server(&ctx, &invalid);
    let result = run(
        &ctx,
        "mcp:check",
        &["--only", "mcp:fake", "--agent", "codex"],
    );
    assert_eq!(result.data["ok"], false);
    assert_eq!(result.data["stage"], "initialize");
    let no_tools = FAKE_SERVER.replace("\"tools\":{}", "");
    fake_server(&ctx, &no_tools);
    let result = run(
        &ctx,
        "mcp:check",
        &["--only", "mcp:fake", "--agent", "codex"],
    );
    assert_eq!(result.data["ok"], true);
    assert_eq!(result.data["tool_count"], Value::Null);
    assert!(result.data["message"]
        .as_str()
        .unwrap()
        .contains("未声明工具能力"));
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn running_mcp_check_holds_account_lock_for_the_entire_server_lifetime() {
    let (root, ctx) = temp_ctx("check-session-lock");
    let body = FAKE_SERVER.replace("read line\necho 'starting fake server'", "while [ ! -f \"$(dirname \"$0\")/continue\" ]; do sleep 0.02; done\nread line\necho 'starting fake server'");
    let server = fake_server(&ctx, &body);
    write_claude(
        &ctx,
        json!({"mcpServers":{"fake":{"command":server.to_str().unwrap()}}}),
    );
    let before = read(&claude_path(&ctx));
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
        .env_clear()
        .env("HOME", &ctx.home_dir)
        .env("USERPROFILE", &ctx.home_dir)
        .env("AGENT_MANAGER_DATA_DIR", &ctx.data_dir)
        .env("PATH", "/usr/bin:/bin")
        .args([
            "mcp:check",
            "--only",
            "mcp:fake",
            "--agent",
            "claude-code",
            "--json",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let marker = server.parent().unwrap().join("ran");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !marker.exists() && std::time::Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if !marker.exists() {
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        panic!(
            "fake server must actually be running: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let args = [
        "--only",
        "mcp:new",
        "--agent",
        "claude-code",
        "--action",
        "add",
        "--spec",
        r#"{"command":"mcp"}"#,
    ];
    let rejected = run(&ctx, "mcp:apply", &args);
    assert!(!rejected.ok);
    assert!(
        errors(&rejected).contains("正在运行"),
        "{}",
        errors(&rejected)
    );
    assert_eq!(read(&claude_path(&ctx)), before);
    std::fs::write(server.parent().unwrap().join("continue"), "continue").unwrap();
    let output = child.wait_with_output().unwrap();
    let outcome: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        output.status.success() && outcome["data"]["ok"] == true,
        "{outcome}"
    );
    apply_ok(&ctx, &args);
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn check_uses_only_selected_account_references_and_sanitized_parent_environment() {
    let (root, ctx) = temp_ctx("check-clean-env");
    let source = configure_secrets_file(&ctx);
    std::fs::write(
        &source,
        "YOUR_MCP_OWN_B=YOUR_FAKE_B_ONLY_VALUE\nYOUR_MCP_A=YOUR_FAKE_A_ONLY_VALUE\n",
    )
    .unwrap();
    let body = format!(
        r#"#!/bin/sh
printf '%s\n' "own=$OWN" "arg=$1" "foreign=${{YOUR_MCP_A-unset}}" "token=${{ANTHROPIC_API_KEY-unset}}" "dir=${{CLAUDE_CONFIG_DIR-unset}}" "endpoint=${{ANTHROPIC_BASE_URL-unset}}" > "$(dirname "$0")/environment"
{}
"#,
        FAKE_SERVER.strip_prefix("#!/bin/sh\n").unwrap()
    );
    let server = fake_server(&ctx, &body);
    write_claude(
        &ctx,
        json!({"mcpServers":{"a":{"command":"mcp","env":{"KEY":"${YOUR_MCP_A}"}}}}),
    );
    write_codex(&ctx, &format!("[mcp_servers.b]\ncommand = {}\nargs = [\"--safe\"]\n[mcp_servers.b.env]\nOWN = \"${{YOUR_MCP_OWN_B}}\"\n", serde_json::to_string(&server.to_string_lossy()).unwrap()));
    let before_a = read(&claude_path(&ctx));
    let before_b = read(&codex_path(&ctx));
    let before_source = read(&source);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
        .env_clear()
        .env("HOME", &ctx.home_dir)
        .env("USERPROFILE", &ctx.home_dir)
        .env("AGENT_MANAGER_DATA_DIR", &ctx.data_dir)
        .env("PATH", "/usr/bin:/bin")
        .env("YOUR_MCP_A", "YOUR_FAKE_PARENT_A_VALUE")
        .env("ANTHROPIC_API_KEY", "YOUR_FAKE_PARENT_TOKEN")
        .env("CLAUDE_CONFIG_DIR", root.join("other-a"))
        .env("ANTHROPIC_BASE_URL", "https://parent-a.example.test")
        .args(["mcp:check", "--only", "mcp:b", "--agent", "codex", "--json"])
        .output()
        .unwrap();
    let outcome: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{}", outcome);
    assert_eq!(outcome["data"]["ok"], true);
    let recorded = read(&server.parent().unwrap().join("environment"));
    assert!(recorded.contains("own=YOUR_FAKE_B_ONLY_VALUE"));
    for line in [
        "foreign=unset",
        "token=unset",
        "dir=unset",
        "endpoint=unset",
    ] {
        assert!(recorded.contains(line), "{recorded}");
    }
    assert!(!recorded.contains("YOUR_FAKE_A"));
    assert_eq!(read(&claude_path(&ctx)), before_a);
    assert_eq!(read(&codex_path(&ctx)), before_b);
    assert_eq!(read(&source), before_source);
    // Own embedded references are supported by the Claude codec too.
    write_claude(
        &ctx,
        json!({"mcpServers":{"own":{"command":server.to_str().unwrap(),"args":["--own=${YOUR_MCP_A}"],"env":{"OWN":"${YOUR_MCP_A}"}}}}),
    );
    let result = run(
        &ctx,
        "mcp:check",
        &["--only", "mcp:own", "--agent", "claude-code"],
    );
    assert!(
        result.ok && result.data["ok"] == true,
        "{}",
        errors(&result)
    );
    assert!(read(&server.parent().unwrap().join("environment"))
        .contains("arg=--own=YOUR_FAKE_A_ONLY_VALUE"));
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn mcp_environment_case_aliases_cannot_override_account_or_process_context() {
    let (root, ctx) = temp_ctx("check-env-case-alias");
    let body = format!(
        "#!/bin/sh\nprintf '%s' \"$OPENAI_API_KEY\" > \"$(dirname \"$0\")/provider-key\"\n{}",
        FAKE_SERVER.strip_prefix("#!/bin/sh\n").unwrap()
    );
    let server = fake_server(&ctx, &body);
    for key in [
        "home",
        "userprofile",
        "homedrive",
        "homepath",
        "codex_home",
        "claude_config_dir",
        "anthropic_config_dir",
        "path",
        "xdg_config_home",
        "systemroot",
        "windir",
        "comspec",
        "pathext",
        "appdata",
        "localappdata",
        "programdata",
        "programfiles",
        "node_options",
        "ld_preload",
        "dyld_insert_libraries",
    ] {
        let unsafe_env = json!({key: root.join("other-account").to_string_lossy()});
        let entry = json!({"command": server.to_str().unwrap(), "env": unsafe_env});
        write_claude(&ctx, json!({"mcpServers": {"unsafe": entry}}));
        let before = read(&claude_path(&ctx));
        let before_db = db_bytes(&ctx);
        let rejected = run(
            &ctx,
            "mcp:check",
            &["--only", "mcp:unsafe", "--agent", "claude-code"],
        );
        assert!(
            !rejected.ok && errors(&rejected).contains("账号目录或进程覆盖"),
            "{key}: {}",
            errors(&rejected)
        );
        let spec = entry.to_string();
        let rejected = apply(
            &ctx,
            &[
                "--only",
                "mcp:new",
                "--agent",
                "claude-code",
                "--action",
                "add",
                "--spec",
                &spec,
            ],
        );
        assert!(
            !rejected.ok && errors(&rejected).contains("账号目录或进程覆盖"),
            "{key}: {}",
            errors(&rejected)
        );
        assert_eq!(read(&claude_path(&ctx)), before);
        assert_eq!(db_bytes(&ctx), before_db);
        assert!(!server.parent().unwrap().join("ran").exists());
    }
    // An explicit own-account provider credential is allowed in the service environment.
    write_claude(&ctx, json!({"mcpServers": {}}));
    let source = configure_secrets_file(&ctx);
    std::fs::write(
        &source,
        "YOUR_MCP_OWN_PROVIDER_KEY=YOUR_FAKE_OWN_PROVIDER_VALUE\n",
    )
    .unwrap();
    let spec = json!({"command": server.to_str().unwrap(), "env": {"OPENAI_API_KEY": "${YOUR_MCP_OWN_PROVIDER_KEY}"}}).to_string();
    apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:own",
            "--agent",
            "claude-code",
            "--action",
            "add",
            "--spec",
            &spec,
        ],
    );
    let result = run(
        &ctx,
        "mcp:check",
        &["--only", "mcp:own", "--agent", "claude-code"],
    );
    assert!(
        result.ok && result.data["ok"] == true,
        "{}",
        errors(&result)
    );
    assert_eq!(
        read(&server.parent().unwrap().join("provider-key")),
        "YOUR_FAKE_OWN_PROVIDER_VALUE"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn account_mcp_references_cannot_use_parent_values_without_an_explicit_source() {
    let (root, ctx) = temp_ctx("check-parent-only-key");
    let body = format!(
        "#!/bin/sh\nprintf '%s' \"$TOKEN\" > \"$(dirname \"$0\")/token\"\n{}",
        FAKE_SERVER.strip_prefix("#!/bin/sh\n").unwrap()
    );
    let server = fake_server(&ctx, &body);
    write_claude(
        &ctx,
        json!({"mcpServers": {"fake": {
            "command": server.to_str().unwrap(), "env": {"TOKEN": "${YOUR_PARENT_ONLY_MCP_KEY}"}
        }}}),
    );
    let before = read(&claude_path(&ctx));
    let spec = r#"{"command":"mcp","env":{"TOKEN":"${YOUR_PARENT_ONLY_MCP_KEY}"}}"#;
    let invoke = |args: &[&str]| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
            .env_clear()
            .env("HOME", &ctx.home_dir)
            .env("USERPROFILE", &ctx.home_dir)
            .env("AGENT_MANAGER_DATA_DIR", &ctx.data_dir)
            .env("PATH", "/usr/bin:/bin")
            .env("YOUR_PARENT_ONLY_MCP_KEY", "YOUR_FAKE_PARENT_ONLY_VALUE")
            .args(args)
            .output()
            .unwrap();
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    for args in [
        vec![
            "mcp:apply",
            "--only",
            "mcp:new",
            "--agent",
            "claude-code",
            "--action",
            "add",
            "--spec",
            spec,
            "--json",
        ],
        vec![
            "mcp:check",
            "--only",
            "mcp:fake",
            "--agent",
            "claude-code",
            "--json",
        ],
    ] {
        let result = invoke(&args);
        assert_eq!(
            result["ok"], false,
            "必须显式选择密钥文件，不能继承同账号变量的父环境取值：{result}"
        );
        assert!(
            result.to_string().contains("显式选择的外部密钥来源"),
            "{result}"
        );
        assert!(!result.to_string().contains("YOUR_FAKE_PARENT_ONLY_VALUE"));
        assert_eq!(read(&claude_path(&ctx)), before);
        assert!(db_bytes(&ctx).is_empty());
        assert!(!server.parent().unwrap().join("ran").exists());
    }
    // Choosing an explicit source enables the same own-account reference.
    let source = configure_secrets_file(&ctx);
    std::fs::write(
        &source,
        "YOUR_PARENT_ONLY_MCP_KEY=YOUR_FAKE_EXPLICIT_SOURCE_VALUE\n",
    )
    .unwrap();
    let result = invoke(&[
        "mcp:check",
        "--only",
        "mcp:fake",
        "--agent",
        "claude-code",
        "--json",
    ]);
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["data"]["ok"], true, "{result}");
    assert!(server.parent().unwrap().join("ran").exists());
    assert_eq!(
        read(&server.parent().unwrap().join("token")),
        "YOUR_FAKE_EXPLICIT_SOURCE_VALUE"
    );
    assert!(!result.to_string().contains("YOUR_FAKE_PARENT_ONLY_VALUE"));
    assert!(!result
        .to_string()
        .contains("YOUR_FAKE_EXPLICIT_SOURCE_VALUE"));
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn check_stops_before_running_when_a_variable_has_no_value() {
    let (root, ctx) = temp_ctx("check-novar");
    configure_secrets_file(&ctx);
    let server = fake_server(&ctx, FAKE_SERVER);
    write_claude(
        &ctx,
        json!({"mcpServers": {"fake": {"command": server.to_str().unwrap(), "env": {"TOKEN": "${AM_CHECK_UNSET_VAR_XYZ}"}}}}),
    );
    let env = run(
        &ctx,
        "mcp:check",
        &["--only", "mcp:fake", "--agent", "claude-code"],
    );
    assert!(env.ok, "{}", errors(&env));
    assert_eq!(env.data["ok"], false);
    assert_eq!(env.data["stage"], "variables");
    assert!(env.data["message"]
        .as_str()
        .unwrap()
        .contains("AM_CHECK_UNSET_VAR_XYZ"));
    assert!(
        !server.parent().unwrap().join("ran").exists(),
        "缺变量时不启动命令"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn check_reports_a_failing_server_without_leaking_the_secret_it_was_given() {
    let (root, ctx) = temp_ctx("check-fail");
    configure_secrets_file(&ctx);
    const SECRET: &str = "plainvalue-qq83hz-moonlight";
    let set = run(
        &ctx,
        "secrets:set",
        &["--var", "AM_CHECK_TOKEN", "--value", SECRET],
    );
    assert!(set.ok, "{}", errors(&set));
    let server = fake_server(
        &ctx,
        "#!/bin/sh\nIFS= read -r initialize || exit 2\necho \"auth failed for token $TOKEN\" >&2\nexit 1\n",
    );
    write_claude(
        &ctx,
        json!({"mcpServers": {"fake": {"command": server.to_str().unwrap(), "env": {"TOKEN": "${AM_CHECK_TOKEN}"}}}}),
    );
    let before_config = read(&claude_path(&ctx));
    let env = run(
        &ctx,
        "mcp:check",
        &["--only", "mcp:fake", "--agent", "claude-code"],
    );
    assert!(env.ok, "{}", errors(&env));
    assert_eq!(env.data["ok"], false);
    let text = env.data.to_string();
    assert!(text.contains("auth failed for token"), "{text}");
    assert_eq!(env.data["message"], "命令退出了，没有回应初始化");
    assert_eq!(env.data["stage"], "initialize");
    assert_eq!(env.data["exit_code"], 1);
    assert!(!text.contains(SECRET), "取值不能出现在结果里：{text}");
    assert_eq!(read(&claude_path(&ctx)), before_config);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn check_rejects_unknown_entries_and_bad_arguments() {
    let (root, ctx) = temp_ctx("check-args");
    write_claude(&ctx, json!({"mcpServers": {}}));
    let missing = run(
        &ctx,
        "mcp:check",
        &["--only", "mcp:nope", "--agent", "claude-code"],
    );
    assert!(!missing.ok);
    assert!(
        errors(&missing).contains("没有 MCP 服务 nope"),
        "{}",
        errors(&missing)
    );
    let bad = run(
        &ctx,
        "mcp:check",
        &["--only", "skill:x", "--agent", "claude-code"],
    );
    assert!(!bad.ok);
    let extra = run(
        &ctx,
        "mcp:check",
        &[
            "--only",
            "mcp:nope",
            "--agent",
            "claude-code",
            "--spec",
            "{}",
        ],
    );
    assert!(!extra.ok);
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn remote_check_disables_curl_defaults_cleans_parent_environment_and_never_sends_foreign_refs() {
    use std::os::unix::fs::PermissionsExt;
    let (root, ctx) = temp_ctx("check-fake-curl");
    let source = configure_secrets_file(&ctx);
    std::fs::write(
        &source,
        "YOUR_REMOTE_A=YOUR_FAKE_REMOTE_A_VALUE\nYOUR_REMOTE_B=YOUR_FAKE_REMOTE_B_VALUE\n",
    )
    .unwrap();
    let bin = ctx.home_dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let curl = bin.join("curl");
    std::fs::write(&curl, r#"#!/bin/sh
[ "$1" = "--disable" ] || exit 10
[ "$2" = "--globoff" ] || exit 11
[ "$3" = "--proto" ] && [ "$4" = "=http,https" ] || exit 12
printf '%s\n' "curlhome=${CURL_HOME-unset}" "xdg=${XDG_CONFIG_HOME-unset}" "foreign=${YOUR_REMOTE_A-unset}" "token=${ANTHROPIC_API_KEY-unset}" > "$(dirname "$0")/env"
cat > "$(dirname "$0")/headers"
body=''
url=''
while [ "$#" -gt 0 ]; do
  if [ "$1" = '--data-binary' ]; then shift; body="$1"; fi
  if [ "$1" = '--url' ]; then shift; url="$1"; fi
  shift
done
[ "$url" = 'https://fake-service.example.test/mcp' ] || exit 13
case "$body" in
  *tools/list*) printf 'HTTP/1.1 200 OK\r\n\r\n%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"fake_remote_tool"}]}}';;
  *notifications/initialized*) printf 'HTTP/1.1 202 Accepted\r\n\r\n';;
  *) printf 'HTTP/1.1 200 OK\r\nMcp-Session-Id: fake-session\r\n\r\n%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fake-curl"}}}';;
esac
"#).unwrap();
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    write_claude(
        &ctx,
        json!({"mcpServers":{"a":{"command":"mcp","env":{"KEY":"${YOUR_REMOTE_A}"}}}}),
    );
    write_codex(&ctx, "[mcp_servers.b]\nurl = \"https://fake-service.example.test/mcp\"\n[mcp_servers.b.env_http_headers]\nAuthorization = \"YOUR_REMOTE_B\"\n");
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"));
    command
        .env_clear()
        .env("HOME", &ctx.home_dir)
        .env("USERPROFILE", &ctx.home_dir)
        .env("AGENT_MANAGER_DATA_DIR", &ctx.data_dir)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("CURL_HOME", root.join("account-a"))
        .env("XDG_CONFIG_HOME", root.join("account-a"))
        .env("YOUR_REMOTE_A", "YOUR_FAKE_PARENT_REMOTE_A")
        .env("ANTHROPIC_API_KEY", "YOUR_FAKE_PARENT_TOKEN")
        .args(["mcp:check", "--only", "mcp:b", "--agent", "codex", "--json"]);
    let output = command.output().unwrap();
    let outcome: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{outcome}");
    assert_eq!(outcome["data"]["ok"], true);
    assert_eq!(outcome["data"]["tools"], json!(["fake_remote_tool"]));
    let headers = read(&bin.join("headers"));
    assert!(headers.contains("Authorization: YOUR_FAKE_REMOTE_B_VALUE"));
    assert!(!headers.contains("YOUR_FAKE_REMOTE_A"));
    let recorded = read(&bin.join("env"));
    for line in [
        "curlhome=unset",
        "xdg=unset",
        "foreign=unset",
        "token=unset",
    ] {
        assert!(recorded.contains(line), "{recorded}");
    }
    std::fs::remove_file(bin.join("headers")).unwrap();
    write_codex(&ctx, "[mcp_servers.b]\nurl = \"https://fake-service.example.test/mcp\"\n[mcp_servers.b.env_http_headers]\nAuthorization = \"YOUR_REMOTE_A\"\n");
    let before = read(&codex_path(&ctx));
    let output = command.output().unwrap();
    assert!(!output.status.success());
    assert!(
        !bin.join("headers").exists(),
        "foreign refs are blocked before any request"
    );
    assert_eq!(read(&codex_path(&ctx)), before);
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn remote_check_rejects_curl_option_urls_protocols_and_header_injection_before_spawn() {
    use std::os::unix::fs::PermissionsExt;
    let (root, ctx) = temp_ctx("check-curl-option-url");
    let source = configure_secrets_file(&ctx);
    let account_a_rc = root.join("account-a-curlrc");
    std::fs::write(
        &account_a_rc,
        "header = \"Authorization: YOUR_FAKE_ACCOUNT_A_HEADER\"\n",
    )
    .unwrap();
    let malicious_url = format!("--config={}", account_a_rc.display());
    std::fs::write(&source, format!("YOUR_REMOTE_BAD_URL={malicious_url}\n")).unwrap();
    let bin = ctx.home_dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let curl = bin.join("curl");
    std::fs::write(
        &curl,
        "#!/bin/sh\ntouch \"$(dirname \"$0\")/ran\"\ncat > \"$(dirname \"$0\")/headers\"\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    let docs = [
        json!({"url": malicious_url}),
        json!({"url": "-K"}),
        json!({"url": "${YOUR_REMOTE_BAD_URL}"}),
        json!({"url": format!("file://{}", account_a_rc.display())}),
        json!({"url": "ftp://127.0.0.1/account-a"}),
        json!({"url": "https://safe.example.test/mcp", "headers": {"X-Safe\nAuthorization": "YOUR_FAKE_ACCOUNT_A_HEADER"}}),
        json!({"url": "https://safe.example.test/mcp", "headers": {"X-Safe": "static\r\nAuthorization: YOUR_FAKE_ACCOUNT_A_HEADER"}}),
    ];
    for doc in docs {
        write_claude(&ctx, json!({"mcpServers": {"unsafe": doc}}));
        let before = read(&claude_path(&ctx));
        let before_db = db_bytes(&ctx);
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
            .env_clear()
            .env("HOME", &ctx.home_dir)
            .env("USERPROFILE", &ctx.home_dir)
            .env("AGENT_MANAGER_DATA_DIR", &ctx.data_dir)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .args([
                "mcp:check",
                "--only",
                "mcp:unsafe",
                "--agent",
                "claude-code",
                "--json",
            ])
            .output()
            .unwrap();
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            result["ok"] == false || result["data"]["ok"] == false,
            "{result}"
        );
        assert!(
            !bin.join("ran").exists(),
            "不安全请求不得启动curl：{result}"
        );
        assert!(
            !bin.join("headers").exists(),
            "不安全请求不得发送header：{result}"
        );
        assert_eq!(read(&claude_path(&ctx)), before);
        assert_eq!(db_bytes(&ctx), before_db);
        assert!(!result.to_string().contains("YOUR_FAKE_ACCOUNT_A_HEADER"));
    }
    let _ = std::fs::remove_dir_all(root);
}

/// 本地假 Streamable HTTP 服务：initialize 回 JSON 并给会话 id；tools/list 要带会话 id 与鉴权头，回 SSE。
/// 返回地址与收到的请求头（小写）记录。
#[cfg(unix)]
fn fake_http_server(
    tools_response: &'static str,
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().take(3) {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut headers = Vec::new();
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let line = line.trim_end().to_string();
                if line.is_empty() {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(value) = lower.strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
                headers.push(lower);
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap();
            log.lock().unwrap().push(headers.join("\n"));
            let response = match body["method"].as_str() {
                Some("initialize") => {
                    let json = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"remote-fake","version":"9"}}}"#;
                    format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: sess-42\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}", json.len())
                }
                Some("tools/list") => {
                    let ok = headers.iter().any(|h| h == "mcp-session-id: sess-42")
                        && headers
                            .iter()
                            .any(|h| h == "authorization: bearer remote-plain-token-77");
                    let json = if ok {
                        tools_response
                    } else {
                        r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32001,"message":"missing session or auth"}}"#
                    };
                    let sse = format!("event: message\ndata: {json}\n\n");
                    format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sse}", sse.len())
                }
                _ => "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_string(),
            };
            stream.write_all(response.as_bytes()).unwrap();
        }
    });
    (url, seen)
}

#[cfg(unix)]
#[test]
fn check_talks_streamable_http_with_session_and_resolved_headers() {
    let (root, ctx) = temp_ctx("check-http");
    configure_secrets_file(&ctx);
    let set = run(
        &ctx,
        "secrets:set",
        &[
            "--var",
            "AM_REMOTE_TOKEN",
            "--value",
            "remote-plain-token-77",
        ],
    );
    assert!(set.ok, "{}", errors(&set));
    // curl must ignore account-external defaults even in the captured HOME.
    std::fs::write(
        ctx.home_dir.join(".curlrc"),
        "header = \"X-Wrong-Account: YOUR_FAKE_CURLRC_A\"\nurl = \"http://127.0.0.1:1/never\"\n",
    )
    .unwrap();
    let (url, seen) =
        fake_http_server(r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"remote_tool"}]}}"#);
    write_claude(
        &ctx,
        json!({"mcpServers": {"remote": {"type": "http", "url": url, "headers": {"Authorization": "Bearer ${AM_REMOTE_TOKEN}"}}}}),
    );
    let env = run(
        &ctx,
        "mcp:check",
        &["--only", "mcp:remote", "--agent", "claude-code"],
    );
    assert!(env.ok, "{}", errors(&env));
    assert_eq!(env.data["ok"], true, "{}", env.data);
    assert_eq!(env.data["server_info"]["name"], "remote-fake");
    assert_eq!(env.data["tools"], json!(["remote_tool"]));
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "initialize、initialized、tools/list 各一次"
    );
    assert!(!env.data.to_string().contains("remote-plain-token-77"));
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .all(|headers| !headers.contains("your_fake_curlrc_a")));
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn check_reports_streamable_http_tools_errors_as_failures() {
    let (root, ctx) = temp_ctx("check-http-tools");
    for response in [
        r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32603,"message":"tools rejected"}}"#,
        r#"{"jsonrpc":"2.0","id":2,"result":{}}"#,
    ] {
        let (url, _) = fake_http_server(response);
        write_claude(
            &ctx,
            json!({"mcpServers":{"remote":{"type":"http","url":url,"headers":{"Authorization":"Bearer remote-plain-token-77"}}}}),
        );
        let result = run(
            &ctx,
            "mcp:check",
            &["--only", "mcp:remote", "--agent", "claude-code"],
        );
        assert!(result.ok, "{}", errors(&result));
        assert_eq!(result.data["ok"], false, "{}", result.data);
        assert_eq!(result.data["stage"], "tools");
        assert!(result.data["tools"].is_null());
    }
    assert!(!ctx.data_dir.exists());
    let _ = std::fs::remove_dir_all(root);
}
