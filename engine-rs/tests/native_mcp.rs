//! MCP 原生域：各助手配置文件里的实际条目即当前配置。
//!
//! 覆盖：只读 status（零写、脱敏、契约形状）、逐条 add/edit/enable/disable/remove/copy、
//! `--base` CAS、Codex 原位编辑保留未建模字段、Claude 停用快照（疑似凭据转存、普通字面量
//! 原样保存）、旧服务库迁移、锁与审计、严格参数。全部在隔离 HOME 与数据目录里跑。

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
    assert!(errors(&unsupported).contains("仅支持 add/edit/copy/remove"));
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
    ctx.home_dir.join(".codex/config.toml")
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
    assert_eq!(fs["actions"], json!(["edit", "disable", "copy", "remove"]));

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
    std::fs::write(bin_dir.join("present-tool"), "#!/bin/sh\n").unwrap();
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
    assert_eq!(c["actions"], json!(["edit", "enable", "copy", "remove"]));
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
    assert_eq!(
        codex_doc(&ctx)["mcp_servers"]["github"]["enabled"].as_bool(),
        Some(true)
    );
    assert_eq!(cell(&ctx, "github", "codex")["state"], "enabled");
    let _ = std::fs::remove_dir_all(root);
}

/// 必测 4（及裁决第 10 条两类）：Claude 停用 = 移出条目 + 快照入库。
/// 疑似凭据的 env 字面量与全部 header 字面量转存到外部密钥文件、快照里只剩 ${VAR}；
/// 普通字面量原样入快照；库中 grep 不到明文；重新启用写回的条目与停用前一致。
#[test]
fn claude_disable_snapshots_entry_transfers_secrets_and_enable_restores_identically() {
    let (root, ctx) = temp_ctx("claude-snapshot");
    let secret = "sk-ant-api03-snapshot0123456789abcdefghijk";
    let header = "Bearer header-secret-value-1234";
    let original_fs = json!({"type": "stdio", "command": "npx", "args": ["-y", "fs"],
        "env": {"AM_T4_API_KEY": secret, "NODE_ENV": "production", "REF": "${AM_T4_REF}"},
        "alwaysLoad": true});
    let original_remote = json!({"type": "http", "url": "https://mcp.example.com/mcp",
        "headers": {"Authorization": header, "X-Ref": "${AM_T4_HDR}"}});
    write_claude(
        &ctx,
        json!({"numStartups": 1, "mcpServers": {"fs": original_fs, "remote": original_remote, "other": {"command": "o"}}}),
    );
    let secrets_file = configure_secrets_file(&ctx);

    let env = apply_ok(
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
    assert!(
        warnings(&env).contains("AM_T4_API_KEY"),
        "{}",
        warnings(&env)
    );
    assert!(!warnings(&env).contains(secret));
    let env = apply_ok(
        &ctx,
        &[
            "--only",
            "mcp:remote",
            "--agent",
            "claude-code",
            "--action",
            "disable",
            "--base",
            &base(&ctx, "remote", "claude-code"),
        ],
    );
    assert!(
        warnings(&env).contains("REMOTE_AUTHORIZATION"),
        "{}",
        warnings(&env)
    );

    let doc: Value = serde_json::from_str(&read(&claude_path(&ctx))).unwrap();
    assert!(doc["mcpServers"].get("fs").is_none(), "条目已移出");
    assert!(doc["mcpServers"].get("remote").is_none());
    assert_eq!(doc["mcpServers"]["other"], json!({"command": "o"}));
    assert_eq!(doc["numStartups"], 1);

    let rows = saved_rows(&ctx);
    let fs_row = rows.iter().find(|r| r.0 == "fs").unwrap();
    assert_eq!(fs_row.1, "claude-code");
    let fs_entry: Value = serde_json::from_str(&fs_row.2).unwrap();
    assert_eq!(fs_entry["env"]["AM_T4_API_KEY"], "${AM_T4_API_KEY}");
    assert_eq!(
        fs_entry["env"]["NODE_ENV"], "production",
        "普通字面量原样入快照"
    );
    assert_eq!(fs_entry["env"]["REF"], "${AM_T4_REF}");
    assert_eq!(fs_row.3, r#"{"env":["AM_T4_API_KEY"],"headers":[]}"#);
    let remote_row = rows.iter().find(|r| r.0 == "remote").unwrap();
    let remote_entry: Value = serde_json::from_str(&remote_row.2).unwrap();
    assert_eq!(
        remote_entry["headers"]["Authorization"],
        "${REMOTE_AUTHORIZATION}"
    );
    assert_eq!(remote_entry["headers"]["X-Ref"], "${AM_T4_HDR}");

    let bytes = db_bytes(&ctx);
    assert!(!contains(&bytes, secret), "数据库不得含明文密钥");
    assert!(
        !contains(&bytes, "header-secret-value-1234"),
        "数据库不得含明文请求头"
    );
    let file = read(&secrets_file);
    assert!(file.contains("AM_T4_API_KEY=") && file.contains(secret));
    assert!(file.contains("REMOTE_AUTHORIZATION="));

    let c = cell(&ctx, "fs", "claude-code");
    assert_eq!(c["state"], "disabled");
    assert_eq!(c["source"], "saved");
    assert_eq!(c["entry"]["env"]["AM_T4_API_KEY"], "${AM_T4_API_KEY}");
    // 不像凭据的字面量照原值显示，界面说得出原值是什么
    assert_eq!(c["entry"]["env"]["NODE_ENV"], "production");
    assert_eq!(c["actions"], json!(["enable", "copy", "remove"]));
    assert_eq!(element(&status(&ctx), "fs")["state"], "disabled");

    let remove_args = [
        "--only",
        "mcp:fs",
        "--agent",
        "claude-code",
        "--action",
        "remove",
        "--base",
        &base(&ctx, "fs", "claude-code"),
    ];
    let before_file = read(&claude_path(&ctx));
    let before_db = db_bytes(&ctx);
    let preview = run(&ctx, "mcp:preview", &remove_args);
    assert!(preview.ok, "{}", errors(&preview));
    assert_eq!(
        preview.data["files"],
        json!([]),
        "已保存的停用项不会改配置文件"
    );
    assert_eq!(read(&claude_path(&ctx)), before_file);
    assert_eq!(db_bytes(&ctx), before_db, "删除预览不得写库");

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
    assert_eq!(claude_entry(&ctx, "fs"), original_fs, "写回与停用前一致");
    assert_eq!(claude_entry(&ctx, "remote"), original_remote);
    assert!(saved_rows(&ctx).is_empty(), "启用后快照删除");
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
        assert!(err.contains("外部密钥文件"), "{err}");
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
            r#"{"display_name":"OpenCode","bin":"opencode","mcp_config":{"path":"~/.config/opencode/opencode.json","format":"opencode"}}"#,
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
    std::env::set_var("AM_T11_TOKEN", "resolved-t11-value");
    std::env::set_var("AM_T11_HDR", "hdr-t11-value");
    let set = run(
        &ctx,
        "policy:set",
        &[
            "--layer",
            "local",
            "--key",
            "agents.definitions.opencode",
            "--value",
            r#"{"display_name":"OpenCode","bin":"opencode","mcp_config":{"path":"~/.config/opencode/opencode.json","format":"opencode"}}"#,
        ],
    );
    assert!(set.ok);
    for agent in ["claude-code", "codex", "opencode"] {
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
                r#"{"command":"npx","args":["-y","pkg"],"env":{"TOKEN":"${AM_T11_TOKEN}","MODE":"fast"}}"#,
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
                r#"{"url":"https://mcp.example.com/mcp","headers":{"Authorization":"${AM_T11_HDR}"}}"#,
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
        "${AM_T11_HDR}"
    );
    assert_eq!(claude_entry(&ctx, "remote1")["type"], "http");
    let doc = codex_doc(&ctx);
    assert_eq!(
        doc["mcp_servers"]["stdio1"]["env"]["MODE"].as_str(),
        Some("fast")
    );
    assert_eq!(
        doc["mcp_servers"]["remote1"]["env_http_headers"]["Authorization"].as_str(),
        Some("AM_T11_HDR")
    );
    let oc: Value = serde_json::from_str(&opencode).unwrap();
    assert_eq!(oc["mcp"]["stdio1"]["command"], json!(["npx", "-y", "pkg"]));
    assert_eq!(
        oc["mcp"]["remote1"]["headers"]["Authorization"],
        "{env:AM_T11_HDR}"
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
    std::env::remove_var("AM_T11_TOKEN");
    std::env::remove_var("AM_T11_HDR");
    let _ = std::fs::remove_dir_all(root);
}

/// 必测 5：复制 Claude → Codex：字段按目标 codec 转换；目标同名默认拒绝；
/// 显式替换需给出目标条目的 base；未建模字段不复制、在 warnings 里列出。
#[test]
fn copy_claude_to_codex_converts_fields_and_guards_existing_target() {
    let (root, ctx) = temp_ctx("copy");
    std::env::set_var("AM_T8_REF", "resolved-t8");
    std::env::set_var("AM_T8_HDR", "hdr-t8");
    write_claude(
        &ctx,
        json!({"mcpServers": {
            "fs": {"type": "stdio", "command": "npx", "args": ["-y", "fs"],
                   "env": {"LIT": "literal-value", "REF": "${AM_T8_REF}"}, "alwaysLoad": true},
            "remote": {"type": "http", "url": "https://mcp.example.com/mcp",
                       "headers": {"X-Ref": "${AM_T8_HDR}", "X-Lit": "static-value"}},
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
    assert_eq!(fs["command"].as_str(), Some("npx"));
    assert_eq!(fs["env"]["LIT"].as_str(), Some("literal-value"));
    assert_eq!(
        fs["env"]["REF"].as_str(),
        Some("resolved-t8"),
        "Codex 不展开 ${{VAR}}，写入时解析"
    );
    assert!(fs.get("alwaysLoad").is_none());
    let remote = &doc["mcp_servers"]["remote"];
    assert_eq!(remote["url"].as_str(), Some("https://mcp.example.com/mcp"));
    assert_eq!(
        remote["env_http_headers"]["X-Ref"].as_str(),
        Some("AM_T8_HDR")
    );
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
    std::env::remove_var("AM_T8_REF");
    std::env::remove_var("AM_T8_HDR");
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
    std::env::set_var("AM_T9_TOKEN", "resolved-t9");
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
    std::env::remove_var("AM_T9_TOKEN");
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

/// 裁决第 8 条：配置备份包含已保存项（快照只含引用与普通字面量），换机导入后可启用。
#[test]
fn saved_items_roundtrip_through_config_backup() {
    let (root, ctx) = temp_ctx("backup-src");
    std::env::set_var("AM_T22_KEY", "sk-ant-api03-backup0123456789abcdefghij");
    write_claude(
        &ctx,
        json!({"mcpServers": {"fs": {"command": "npx", "args": ["-y", "fs"],
            "env": {"AM_T22_KEY": "sk-ant-api03-backup0123456789abcdefghij", "NODE_ENV": "production"}}}}),
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
    let imported = run(&ctx2, "config:import", &["--value", &payload]);
    assert!(imported.ok, "{}", errors(&imported));
    let again = run(&ctx2, "config:export", &[]);
    assert_eq!(
        again.data["mcp_saved"], exported.data["mcp_saved"],
        "往返一致"
    );
    // 新机器上变量由进程环境提供，启用写回原值
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
        json!({"AM_T22_KEY": "sk-ant-api03-backup0123456789abcdefghij", "NODE_ENV": "production"})
    );
    // 带明文的已保存项拒绝导入
    let bad = payload.replace("${AM_T22_KEY}", "sk-ant-api03-injected0123456789abcdef");
    let (root3, ctx3) = temp_ctx("backup-bad");
    let rejected = run(&ctx3, "config:import", &["--value", &bad]);
    assert!(!rejected.ok, "含疑似凭据的已保存项必须拒绝");
    std::env::remove_var("AM_T22_KEY");
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
