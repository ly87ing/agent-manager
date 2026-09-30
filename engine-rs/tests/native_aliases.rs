// Unix 的 rc 文件（~/.zshrc / ~/.zprofile）；Windows 的命令脚本见 native_aliases_windows.rs。
#![cfg(unix)]

//! aliases 域：~/.zshrc 与 ~/.zprofile 受管区块里的实际行为准 + 内置推荐 + 旧版本保存的别名。
//!
//! status 输出区块内的实际别名（按名合并，标出在哪些文件）、区块外用户自己的别名（只读）与
//! 每个文件的 base；apply 按名编辑区块、每个文件独立 base CAS；内置别名只作推荐；旧库
//! alias_sources 里没生效的别名作为建议出现，可加入或丢弃。

use agent_manager_engine::{execute, store, Context, Envelope, Request};
use fs2::FileExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::path::PathBuf;

const BEGIN: &str = "# >>> agent-manager shell aliases >>>";
const END: &str = "# <<< agent-manager shell aliases <<<";

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    // 本文件按 zsh 用户断言（主目标 ~/.zshrc）；bash 用户见 native_aliases_bash.rs。各用例设同一个值，并发无妨
    std::env::set_var("SHELL", "/bin/zsh");
    // 锁状态只由 execute 从 AGENT_MANAGER_LOCK_* 推导，外部环境变量不得污染断言。
    std::env::remove_var("AGENT_MANAGER_LOCK_HELD");
    std::env::remove_var("AGENT_MANAGER_LOCK_PID");
    let base = std::env::temp_dir().join(format!(
        "am-native-aliases-test-{}-{}-{:x}",
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
        run_id: "aliases-test-run".to_string(),
    }
}

fn run(ctx: &Context, command: &str, args: &[&str]) -> Envelope {
    execute(ctx, &request(command, args))
}

fn sha(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn status(ctx: &Context) -> Value {
    let env = run(ctx, "aliases:status", &[]);
    assert!(env.ok, "{:?}", env.errors);
    env.data
}

fn element(status: &Value, id: &str) -> Value {
    status["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("缺元素 {id}"))
}

fn base_of(status: &Value, id: &str) -> String {
    element(status, id)["observed"]["base"]
        .as_str()
        .unwrap_or_else(|| panic!("{id} 没有 base"))
        .to_string()
}

/// 两个文件都带上当前 base 的写入参数。
fn both_bases(status: &Value) -> Vec<String> {
    vec![
        "--zshrc-base".into(),
        base_of(status, "aliases:zshrc"),
        "--zprofile-base".into(),
        base_of(status, "aliases:zprofile"),
    ]
}

fn apply(ctx: &Context, bases: &[String], op: &[&str]) -> Envelope {
    let mut args: Vec<&str> = bases.iter().map(String::as_str).collect();
    args.extend_from_slice(op);
    run(ctx, "aliases:apply", &args)
}

fn read(ctx: &Context, rel: &str) -> Option<String> {
    std::fs::read_to_string(ctx.home_dir.join(rel)).ok()
}

fn write(ctx: &Context, rel: &str, text: &str) {
    let path = ctx.home_dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn block(lines: &[&str]) -> String {
    format!("{BEGIN}\n{}\n{END}\n", lines.join("\n"))
}

fn names(items: &Value) -> Vec<String> {
    items
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap().to_string())
        .collect()
}

fn count(ctx: &Context, sql: &str) -> i64 {
    let conn = store::open_readonly(ctx).unwrap().expect("db must exist");
    conn.query_row(sql, [], |row| row.get(0)).unwrap_or(0)
}

/// 旧版本的 alias_sources 行（经旧版本 sources:set 写入的形状）。
fn seed_legacy(ctx: &Context, value: &Value) {
    let conn = store::open_write(ctx).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS alias_sources (
            key TEXT PRIMARY KEY NOT NULL,
            value_json TEXT NOT NULL,
            revision TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .unwrap();
    let text = value.to_string();
    conn.execute(
        "INSERT OR REPLACE INTO alias_sources VALUES ('aliases.shell', ?1, ?2, '2026-09-01T00:00:00Z')",
        rusqlite::params![text, sha(&text)],
    )
    .unwrap();
}

/// 与 agent-skills 的 aliases/aliases.json 相同结构的一份来源别名。
const ALIAS_SOURCE: &str = r#"{
  "timezone": "America/Los_Angeles",
  "lang": "en_US.UTF-8",
  "aliases": {
    "claude": "TZ=\"{timezone}\" LANG=\"{lang}\" LC_ALL=\"{lang}\" claude",
    "codex": "TZ=\"{timezone}\" LANG=\"{lang}\" LC_ALL=\"{lang}\" codex",
    "grok": "TZ=\"{timezone}\" LANG=\"{lang}\" LC_ALL=\"{lang}\" grok",
    "chrome": "open --env TZ=\"{timezone}\" --env LANG=\"{lang}\" --env LC_ALL=\"{lang}\" -n -a \"Google Chrome\" --args --lang=en-US"
  }
}"#;

/// 登记一个本地目录技能来源，目录下放 aliases/*.json。返回来源根目录。
fn add_alias_source(ctx: &Context, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = ctx
        .data_dir
        .parent()
        .unwrap()
        .join(format!("source-{name}"));
    for (file, body) in files {
        let path = dir.join("aliases").join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    let env = run(
        ctx,
        "policy:set",
        &[
            "--layer",
            "local",
            "--key",
            &format!("skills.repositories.{name}"),
            "--value",
            &json!({ "source_path": dir.to_string_lossy() }).to_string(),
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    dir
}

/// 空环境：status 零写；两文件都没有区块（missing，base=missing，可写）；程序不带推荐别名，
/// 只给推荐来源；审计把「没有区块」当正常空状态（notice），不算待生成。
#[test]
fn fresh_status_is_zero_write_and_only_suggests_a_source() {
    let (root, ctx) = temp_ctx("fresh");
    let env = run(&ctx, "aliases:status", &["--offline"]);
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.mode.offline);
    assert!(!env.lock.held);
    let data = &env.data;
    let elements = data["elements"].as_array().unwrap();
    // zsh 用户：~/.zshrc 为主目标，另列 ~/.zprofile 与 ~/.bashrc（有本工具那一段才同步）
    assert_eq!(elements.len(), 3);
    assert_eq!(elements[0]["id"], "aliases:zshrc");
    assert_eq!(elements[1]["id"], "aliases:zprofile");
    assert_eq!(elements[2]["id"], "aliases:bashrc");
    assert_eq!(elements[0]["spec"]["target"], "~/.zshrc");
    assert_eq!(elements[1]["spec"]["target"], "~/.zprofile");
    assert_eq!(elements[2]["spec"]["target"], "~/.bashrc");
    for el in elements {
        assert_eq!(el["state"], "missing");
        assert_eq!(el["observed"]["base"], "missing");
        assert_eq!(el["observed"]["target_exists"], false);
        assert_eq!(el["observed"]["entries"], json!([]));
        assert_eq!(el["observed"]["external"], json!([]));
        assert_eq!(el["actions"][0]["id"], "write");
    }
    assert_eq!(data["aliases"], json!([]));
    assert_eq!(
        data["alias_recommendations"],
        json!([]),
        "推荐别名不编进程序"
    );
    assert_eq!(data["alias_suggested_sources"][0]["name"], "agent-skills");
    assert_eq!(
        data["alias_suggested_sources"][0]["collection"],
        Value::Null
    );
    assert_eq!(data["alias_legacy"], json!([]));

    // 契约：status 顶层只用已声明的键
    let schema: Value = serde_json::from_str(
        &std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../contract/engine-v1/status.schema.json"),
        )
        .unwrap(),
    )
    .unwrap();
    for key in data.as_object().unwrap().keys() {
        assert!(schema["properties"].get(key).is_some(), "{key}");
    }

    let audit = run(&ctx, "system:audit", &[]);
    for entry in audit.data["sections"]["aliases"].as_array().unwrap() {
        assert_eq!(entry["status"], "notice", "{entry}");
    }
    assert!(!ctx.data_dir.exists(), "status 不得创建 data_dir/DB");
    assert!(!ctx.home_dir.exists(), "status 不得创建 home 或 rc 文件");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn preview_matches_both_rc_files_and_does_not_write() {
    let (root, ctx) = temp_ctx("preview");
    let original = format!(
        "# my comment\nalias own='echo own'\n{}",
        block(&["alias old='old'"])
    );
    write(&ctx, ".zshrc", &original);
    let bases = both_bases(&status(&ctx));
    let mut args: Vec<&str> = bases.iter().map(String::as_str).collect();
    args.extend_from_slice(&[
        "--set",
        "new",
        "--value",
        "echo new",
        "--rename-from",
        "old",
    ]);
    let preview = run(&ctx, "aliases:preview", &args);
    assert!(preview.ok, "{:?}", preview.errors);
    assert!(!preview.lock.held);
    let files = preview.data["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "~/.zprofile 没有受管区块：不新建");
    assert_eq!(preview.data["skipped"], json!(["aliases:zprofile"]));
    assert_eq!(files[0]["target"], "aliases:zshrc");
    assert_eq!(files[0]["exists"], true);
    assert_eq!(
        files[0]["path"],
        std::fs::canonicalize(ctx.home_dir.join(".zshrc"))
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    assert!(files[0]["content"]
        .as_str()
        .unwrap()
        .contains("# my comment\nalias own='echo own'"));
    assert!(!files[0]["content"].as_str().unwrap().contains("alias old="));
    assert_eq!(read(&ctx, ".zshrc").as_deref(), Some(original.as_str()));
    assert_eq!(read(&ctx, ".zprofile"), None);
    assert!(!ctx.data_dir.exists(), "预览不得建库");

    let applied = run(&ctx, "aliases:apply", &args);
    assert!(applied.ok, "{:?}", applied.errors);
    assert_eq!(
        read(&ctx, ".zshrc").as_deref(),
        files[0]["content"].as_str()
    );
    assert_eq!(read(&ctx, ".zprofile"), None);
    let stale = run(&ctx, "aliases:preview", &args);
    assert!(!stale.ok);
    assert!(stale.errors[0].message.contains("已在外部修改"));
    let _ = std::fs::remove_dir_all(&root);
}

/// 只加本工具的那几行：已有 ~/.zshrc 末尾直接接上区块，不另插空行；~/.zprofile 已存在但
/// 没有受管区块时一个字节都不动。
#[test]
fn first_alias_appends_block_only_and_leaves_zprofile_alone() {
    let (root, ctx) = temp_ctx("append-only");
    write(&ctx, ".zshrc", "# user\nalias ll=\"ls -l\"\n");
    write(&ctx, ".zprofile", "export PATH=\"$HOME/bin:$PATH\"\n");
    let env = apply(
        &ctx,
        &both_bases(&status(&ctx)),
        &["--set", "cc2", "--value", "claude"],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["skipped"], json!(["aliases:zprofile"]));
    assert_eq!(
        read(&ctx, ".zshrc").unwrap(),
        format!(
            "# user\nalias ll=\"ls -l\"\n{}",
            block(&["alias cc2='claude'"])
        )
    );
    assert_eq!(
        read(&ctx, ".zprofile").unwrap(),
        "export PATH=\"$HOME/bin:$PATH\"\n"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 来源别名：按来源里 aliases/*.json 推荐，带上来源名；同名取先读到的文件，结构不合法的
/// 文件整份跳过，README 不算；有来源别名时不再给推荐来源。
#[test]
fn source_aliases_are_recommended_with_their_source() {
    let (root, ctx) = temp_ctx("source");
    add_alias_source(
        &ctx,
        "mine",
        &[
            ("aliases.json", ALIAS_SOURCE),
            (
                "b-more.json",
                r#"{"aliases": {"claude": "claude-other", "ll": "ls -l"}}"#,
            ),
            ("c-bad.json", r#"{"aliases": {"bad": "has'quote"}}"#),
            ("README.md", "说明\n"),
        ],
    );
    let data = status(&ctx);
    assert_eq!(
        names(&data["alias_recommendations"]),
        ["chrome", "claude", "codex", "grok", "ll"]
    );
    let claude = &data["alias_recommendations"][1];
    assert_eq!(
        claude["line"],
        "alias claude='TZ=\"America/Los_Angeles\" LANG=\"en_US.UTF-8\" LC_ALL=\"en_US.UTF-8\" claude'",
        "同名取先读到的文件"
    );
    assert_eq!(claude["source"], "mine");
    assert_eq!(data["alias_suggested_sources"], json!([]));
    let _ = std::fs::remove_dir_all(&root);
}

/// 添加一条推荐：只在 ~/.zshrc 出现区块（新文件 0600），不新建 ~/.zprofile；推荐列表里不再有它；
/// 审计不含取值；写出的内容能过 zsh 语法检查（本机有 zsh 时）。
#[test]
fn adding_a_recommendation_writes_zshrc_only_and_passes_zsh_syntax() {
    let (root, ctx) = temp_ctx("recommend");
    add_alias_source(&ctx, "mine", &[("aliases.json", ALIAS_SOURCE)]);
    let before = status(&ctx);
    let db_before = std::fs::read(store::db_path(&ctx)).unwrap();
    let bases = both_bases(&before);
    let mut preview_args: Vec<&str> = bases.iter().map(String::as_str).collect();
    preview_args.extend_from_slice(&["--recommended", "claude"]);
    let preview = run(&ctx, "aliases:preview", &preview_args);
    assert!(preview.ok, "{:?}", preview.errors);
    assert!(!preview.lock.held);
    assert_eq!(preview.data["files"].as_array().unwrap().len(), 1);
    assert!(read(&ctx, ".zshrc").is_none());
    assert!(read(&ctx, ".zprofile").is_none());
    assert_eq!(std::fs::read(store::db_path(&ctx)).unwrap(), db_before);
    let env = apply(&ctx, &both_bases(&before), &["--recommended", "claude"]);
    assert!(env.ok, "{:?}", env.errors);
    assert!(env.lock.held, "真实写入必须持锁");
    assert_eq!(env.data["steps"].as_array().unwrap().len(), 1);
    let expected = block(&[
        "alias claude='TZ=\"America/Los_Angeles\" LANG=\"en_US.UTF-8\" LC_ALL=\"en_US.UTF-8\" claude'",
    ]);
    assert_eq!(read(&ctx, ".zshrc").unwrap(), expected);
    assert!(read(&ctx, ".zprofile").is_none(), "不新建 ~/.zprofile");
    assert_eq!(
        read(&ctx, ".zshrc").as_deref(),
        preview.data["files"][0]["content"].as_str()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(ctx.home_dir.join(".zshrc"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "新文件必须为 0600");
    }
    let after = status(&ctx);
    assert_eq!(names(&after["aliases"]), ["claude"]);
    assert_eq!(after["aliases"][0]["targets"], json!(["aliases:zshrc"]));
    assert_eq!(after["aliases"][0]["consistent"], true);
    assert_eq!(
        names(&after["alias_recommendations"]),
        ["chrome", "codex", "grok"]
    );
    assert_eq!(element(&after, "aliases:zshrc")["state"], "managed-ok");
    assert_eq!(element(&after, "aliases:zprofile")["state"], "missing");

    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let detail: String = conn
        .query_row(
            "SELECT detail_json FROM operation_logs WHERE operation_type = 'aliases:apply'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(detail.contains("\"recommended\""), "{detail}");
    assert!(
        detail.contains("\"source\":\"mine\""),
        "审计记来源: {detail}"
    );
    assert!(!detail.contains("Los_Angeles"), "审计不得含取值: {detail}");

    // 再添加同一条推荐：它已不在推荐里，拒绝且文件不变
    let env = apply(&ctx, &both_bases(&after), &["--recommended", "claude"]);
    assert!(!env.ok);
    assert!(env.errors[0].message.contains("推荐已不适用"));
    assert_eq!(read(&ctx, ".zshrc").unwrap(), expected);

    if let Ok(out) = std::process::Command::new("zsh")
        .arg("-n")
        .arg(ctx.home_dir.join(".zshrc"))
        .output()
    {
        assert!(
            out.status.success(),
            "zsh -n 失败: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 外部在区块内改了一条：status 如实展示新值；拿旧 base 写入被拒，两个文件都不变。
#[test]
fn external_edit_is_shown_and_stale_base_is_rejected_for_every_file() {
    let (root, ctx) = temp_ctx("external");
    let original = block(&["alias ll='ls -l'"]);
    write(&ctx, ".zshrc", &original);
    write(&ctx, ".zprofile", &original);
    let opened = status(&ctx);
    assert_eq!(opened["aliases"][0]["value"], "ls -l");

    let changed = block(&["alias ll='ls -alh'"]);
    write(&ctx, ".zprofile", &changed);
    let now = status(&ctx);
    let zprofile = element(&now, "aliases:zprofile");
    assert_eq!(zprofile["observed"]["entries"][0]["value"], "ls -alh");
    assert_eq!(
        now["aliases"][0]["consistent"], false,
        "两文件取值不同要标出"
    );

    // zshrc 的 base 还对得上，zprofile 的不对：整体拒绝，一个字节都不写
    let env = apply(
        &ctx,
        &both_bases(&opened),
        &["--set", "gs", "--value", "git status"],
    );
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "ALIAS_ERROR");
    assert!(
        env.errors[0].message.contains("已在外部修改"),
        "{:?}",
        env.errors
    );
    assert!(!env.lock.held, "预检即拒绝，不得取锁");
    assert_eq!(read(&ctx, ".zshrc").unwrap(), original);
    assert_eq!(read(&ctx, ".zprofile").unwrap(), changed);
    assert!(!store::db_path(&ctx).exists(), "拒绝路径不得建库");

    // 带新 base 写入成功，外部改的那条原样保留
    let env = apply(
        &ctx,
        &both_bases(&now),
        &["--set", "gs", "--value", "git status"],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        read(&ctx, ".zprofile").unwrap(),
        block(&["alias ll='ls -alh'", "alias gs='git status'"])
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 区块外用户自己的 alias 只读列出、标为其他来源；编辑区块后区块外字节不变；同名的
/// 内置别名不再推荐。
#[test]
fn outside_aliases_are_listed_read_only_and_never_touched() {
    let (root, ctx) = temp_ctx("outside");
    add_alias_source(&ctx, "mine", &[("aliases.json", ALIAS_SOURCE)]);
    let pre = "# 我的配置\nalias claude='my-own-claude'\nexport PATH=/opt/bin:$PATH\n";
    let post = "alias gp=\"git push\"\nbindkey '^R' history-incremental-search-backward\n";
    write(
        &ctx,
        ".zshrc",
        &format!("{pre}{}{post}", block(&["alias ll='ls -l'"])),
    );
    let before = status(&ctx);
    let zshrc = element(&before, "aliases:zshrc");
    assert_eq!(names(&zshrc["observed"]["external"]), ["claude", "gp"]);
    assert_eq!(zshrc["observed"]["external"][1]["value"], Value::Null);
    assert_eq!(
        names(&before["aliases"]),
        ["ll"],
        "区块外的不算本工具的别名"
    );
    assert!(
        !names(&before["alias_recommendations"]).contains(&"claude".to_string()),
        "已有同名别名时不推荐"
    );

    for op in [
        vec!["--set", "ll", "--value", "ls -alh"],
        vec!["--set", "gs", "--value", "git status"],
        vec!["--remove", "ll"],
    ] {
        let now = status(&ctx);
        let env = apply(
            &ctx,
            &["--zshrc-base".into(), base_of(&now, "aliases:zshrc")],
            &op,
        );
        assert!(env.ok, "{op:?}: {:?}", env.errors);
        let text = read(&ctx, ".zshrc").unwrap();
        assert!(text.starts_with(pre), "区块前字节必须原样: {text}");
        assert!(text.ends_with(post), "区块后字节必须原样: {text}");
    }
    assert_eq!(
        read(&ctx, ".zshrc").unwrap(),
        format!("{pre}{}{post}", block(&["alias gs='git status'"]))
    );
    assert!(
        read(&ctx, ".zprofile").is_none(),
        "没给 zprofile 的 base 就不写它"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 删掉最后一条：整个区块连同两行注释移除；区块里其他写法的行与注释在按名编辑时原样保留。
#[test]
fn edit_keeps_foreign_lines_and_empty_block_is_removed() {
    let (root, ctx) = temp_ctx("remove-block");
    write(
        &ctx,
        ".zshrc",
        &format!(
            "pre\n{}post\n",
            block(&["# 手写注释", "alias ll=\"ls -l\"", "alias gs='git status'"])
        ),
    );
    let now = status(&ctx);
    let entries = &element(&now, "aliases:zshrc")["observed"]["entries"];
    assert_eq!(entries[0]["value"], Value::Null, "非本工具写法取值为 null");
    assert_eq!(entries[0]["line"], "alias ll=\"ls -l\"");

    // 改名：gs → gst，位置不变，其余行不动
    let env = apply(
        &ctx,
        &["--zshrc-base".into(), base_of(&now, "aliases:zshrc")],
        &[
            "--set",
            "gst",
            "--value",
            "git status -sb",
            "--rename-from",
            "gs",
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(
        read(&ctx, ".zshrc").unwrap(),
        format!(
            "pre\n{}post\n",
            block(&[
                "# 手写注释",
                "alias ll=\"ls -l\"",
                "alias gst='git status -sb'"
            ])
        )
    );
    for name in ["ll", "gst"] {
        let now = status(&ctx);
        let env = apply(
            &ctx,
            &["--zshrc-base".into(), base_of(&now, "aliases:zshrc")],
            &["--remove", name],
        );
        assert!(env.ok, "{:?}", env.errors);
    }
    // 只剩一行注释：不算空，区块保留
    assert_eq!(
        read(&ctx, ".zshrc").unwrap(),
        format!("pre\n{}post\n", block(&["# 手写注释"]))
    );

    // 真空了才整块移除
    write(
        &ctx,
        ".zshrc",
        &format!("pre\n{}post\n", block(&["alias ll='ls -l'"])),
    );
    let now = status(&ctx);
    let args = ["--zshrc-base".to_string(), base_of(&now, "aliases:zshrc")];
    let preview = run(
        &ctx,
        "aliases:preview",
        &["--zshrc-base", &args[1], "--remove", "ll"],
    );
    assert!(preview.ok, "{:?}", preview.errors);
    assert!(!preview.lock.held);
    assert_eq!(preview.data["files"][0]["content"], "pre\npost\n");
    assert_eq!(
        read(&ctx, ".zshrc").unwrap(),
        format!("pre\n{}post\n", block(&["alias ll='ls -l'"]))
    );
    let env = apply(&ctx, &args, &["--remove", "ll"]);
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(read(&ctx, ".zshrc").unwrap(), "pre\npost\n");
    assert_eq!(
        read(&ctx, ".zshrc").as_deref(),
        preview.data["files"][0]["content"].as_str()
    );
    let after = status(&ctx);
    assert_eq!(element(&after, "aliases:zshrc")["state"], "missing");
    assert_eq!(base_of(&after, "aliases:zshrc"), "missing");

    // 不存在的文件上删除：no-op，不建文件，也不记审计
    let audits = count(&ctx, "SELECT COUNT(*) FROM operation_logs");
    let env = apply(
        &ctx,
        &["--zprofile-base".into(), "missing".into()],
        &["--remove", "ll"],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["steps"], json!([]));
    assert_eq!(env.data["skipped"], json!(["aliases:zprofile"]));
    assert!(read(&ctx, ".zprofile").is_none());
    assert_eq!(count(&ctx, "SELECT COUNT(*) FROM operation_logs"), audits);
    let _ = std::fs::remove_dir_all(&root);
}

/// 取值校验：单引号/换行/NUL/疑似凭据一律拒绝且零写，错误不回显取值；非法名称是参数错误。
#[test]
fn invalid_values_and_names_are_rejected_without_writes_or_echo() {
    let (root, ctx) = temp_ctx("invalid");
    let marker = "VALUE-MARKER-51c2";
    for value in [
        format!("inject'{marker}"),
        format!("line1\n{marker}"),
        format!("nul\0{marker}"),
        format!("curl -H \"Authorization: Bearer sk-abcdefghijklmnopqrstuvwxyz123456\" {marker}"),
    ] {
        let env = run(
            &ctx,
            "aliases:apply",
            &["--zshrc-base", "missing", "--set", "x", "--value", &value],
        );
        assert!(!env.ok, "{value:?} 应被拒绝");
        assert_eq!(env.errors[0].code, "ALIAS_ERROR");
        assert!(!serde_json::to_string(&env).unwrap().contains(marker));
    }
    for (command, args) in [
        ("aliases:status", vec!["--only", "aliases:zshrc"]),
        ("aliases:status", vec!["--dry-run"]),
        ("aliases:apply", vec![]),
        ("aliases:apply", vec!["--only", "aliases:zshrc"]),
        (
            "aliases:apply",
            vec![
                "--zshrc-base",
                "missing",
                "--allow-takeover",
                "--remove",
                "x",
            ],
        ),
        (
            "aliases:apply",
            vec!["--zshrc-base", "missing", "--dry-run", "--remove", "x"],
        ),
        (
            "aliases:apply",
            vec![
                "--zshrc-base",
                "missing",
                "--set",
                "bad name",
                "--value",
                "x",
            ],
        ),
        (
            "aliases:apply",
            vec!["--zshrc-base", marker, "--remove", "x"],
        ),
        ("aliases:discard-legacy", vec!["--legacy", marker]),
    ] {
        let env = run(&ctx, command, &args);
        assert!(!env.ok, "{command} {args:?} 应失败");
        assert_eq!(env.errors[0].code, "USAGE_ERROR", "{command} {args:?}");
        assert!(!serde_json::to_string(&env).unwrap().contains(marker));
    }
    assert!(!ctx.data_dir.exists(), "拒绝路径不得建库或锁文件");
    assert!(!ctx.home_dir.exists(), "拒绝路径不得写 rc 文件");
    let _ = std::fs::remove_dir_all(&root);
}

/// 并发第二个 apply 被全局锁拒绝，零写；释放后照常写入。
#[test]
fn concurrent_apply_is_refused_by_lock() {
    let (root, ctx) = temp_ctx("lock");
    std::fs::create_dir_all(&ctx.data_dir).unwrap();
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(ctx.data_dir.join("agent-manager.apply.lock"))
        .unwrap();
    lock_file.lock_exclusive().unwrap();
    let args = ["--zshrc-base", "missing", "--set", "ll", "--value", "ls -l"];
    let env = run(&ctx, "aliases:apply", &args);
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "ALIAS_ERROR");
    assert!(!env.lock.held);
    assert!(!ctx.home_dir.exists(), "锁拒绝后不得写任何目标文件");
    drop(lock_file);
    let env = run(&ctx, "aliases:apply", &args);
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(read(&ctx, ".zshrc").unwrap(), block(&["alias ll='ls -l'"]));
    let _ = std::fs::remove_dir_all(&root);
}

/// 审计写入失败：两个已写文件都回滚到原样（包括新建的文件被删掉）。
#[test]
fn late_failure_rolls_back_every_written_file() {
    let (root, ctx) = temp_ctx("rollback");
    let original = format!("export A=1\n{}", block(&["alias ll='ls -l'"]));
    write(&ctx, ".zshrc", &original);
    {
        let conn = store::open_write(&ctx).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_operation_logs BEFORE INSERT ON operation_logs
             BEGIN SELECT RAISE(ABORT, 'injected-end-failure'); END;",
        )
        .unwrap();
    }
    let now = status(&ctx);
    let env = apply(
        &ctx,
        &both_bases(&now),
        &["--set", "gs", "--value", "git status"],
    );
    assert!(!env.ok);
    assert!(env.lock.held, "已进入真实 apply");
    assert!(env.errors[0].message.contains("injected-end-failure"));
    assert_eq!(read(&ctx, ".zshrc").unwrap(), original, "已写文件必须回滚");
    assert!(read(&ctx, ".zprofile").is_none(), "新建的文件必须删掉");
    let _ = std::fs::remove_dir_all(&root);
}

/// 不安全或读不懂的目标（断开的符号链接、marker 不成对）只报原因、不给写入；给了它的 base 也拒绝。
#[cfg(unix)]
#[test]
fn unsafe_or_malformed_targets_are_blocked_without_writes() {
    let (root, ctx) = temp_ctx("blocked");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let elsewhere = root.join("elsewhere-zshrc");
    std::os::unix::fs::symlink(&elsewhere, ctx.home_dir.join(".zshrc")).unwrap();
    let broken = format!("{BEGIN}\nalias ll='ls -l'\n");
    write(&ctx, ".zprofile", &broken);

    let now = status(&ctx);
    for id in ["aliases:zshrc", "aliases:zprofile"] {
        let el = element(&now, id);
        assert_eq!(el["state"], "unmanaged-detected");
        assert!(el["observed"]["blocked_reason"].as_str().is_some());
        assert_eq!(el["observed"]["base"], Value::Null);
        assert_eq!(el["actions"], json!([]));
    }
    assert!(
        element(&now, "aliases:zprofile")["observed"]["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("不成对")
    );
    for flag in ["--zshrc-base", "--zprofile-base"] {
        let env = run(
            &ctx,
            "aliases:apply",
            &[flag, "missing", "--set", "x", "--value", "1"],
        );
        assert!(!env.ok);
        assert!(
            env.errors[0].message.contains("不能读写"),
            "{:?}",
            env.errors
        );
    }
    assert!(!elsewhere.exists(), "断开的链接不得被写出目标文件");
    assert_eq!(read(&ctx, ".zprofile").unwrap(), broken);
    assert!(!store::db_path(&ctx).exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// rc 文件是指向 dotfiles 仓库的符号链接时，与 hooks、接入配置一致：读写最终目标文件，
/// 链接本身不变；目标文件在读取后被外部改过时，旧 base 拒绝。
#[cfg(unix)]
#[test]
fn symlinked_rc_files_are_read_and_written_through_to_the_real_file() {
    let (root, ctx) = temp_ctx("symlink-through");
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let dotfiles = root.join("dotfiles");
    std::fs::create_dir_all(&dotfiles).unwrap();
    let real = dotfiles.join("zshrc");
    std::fs::write(&real, b"# dotfiles zshrc\nexport EDITOR=vim\n").unwrap();
    let link = ctx.home_dir.join(".zshrc");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let now = status(&ctx);
    let el = element(&now, "aliases:zshrc");
    assert!(
        el["observed"]["blocked_reason"].is_null(),
        "指向普通文件的链接不再阻塞: {el}"
    );
    let base = el["observed"]["base"].as_str().unwrap().to_string();
    let env = run(
        &ctx,
        "aliases:apply",
        &[
            "--zshrc-base",
            &base,
            "--set",
            "gs",
            "--value",
            "git status",
        ],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "链接本身必须保持为链接"
    );
    assert_eq!(std::fs::read_link(&link).unwrap(), real);
    let text = std::fs::read_to_string(&real).unwrap();
    assert!(
        text.starts_with("# dotfiles zshrc\nexport EDITOR=vim\n"),
        "{text}"
    );
    assert!(text.contains("alias gs='git status'"), "{text}");

    // 目标文件被外部改过：拿旧 base 写入被拒，目标字节不变
    let stale = element(&status(&ctx), "aliases:zshrc")["observed"]["base"]
        .as_str()
        .unwrap()
        .to_string();
    let edited = text.replace("git status", "git status -sb");
    std::fs::write(&real, &edited).unwrap();
    let env = run(
        &ctx,
        "aliases:apply",
        &["--zshrc-base", &stale, "--remove", "gs"],
    );
    assert!(!env.ok);
    assert!(
        env.errors[0].message.contains("已在外部修改"),
        "{:?}",
        env.errors
    );
    assert_eq!(std::fs::read_to_string(&real).unwrap(), edited);

    // 指向目录的链接仍然拒绝
    let (root2, ctx2) = temp_ctx("symlink-dir");
    std::fs::create_dir_all(&ctx2.home_dir).unwrap();
    let dir = root2.join("a-directory");
    std::fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(&dir, ctx2.home_dir.join(".zshrc")).unwrap();
    let el = element(&status(&ctx2), "aliases:zshrc");
    assert!(el["observed"]["blocked_reason"].as_str().is_some(), "{el}");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&root2);
}

/// 旧库迁移：用户存过、与内置不同、实际区块里没有的别名显示为建议；与内置相同或已在区块里
/// 的不提示；同名不同值时带上实际那一行。加入后出现在区块里、库里该条删掉；丢弃只删库。
#[test]
fn legacy_saved_aliases_become_suggestions_that_can_be_added_or_discarded() {
    let (root, ctx) = temp_ctx("legacy");
    seed_legacy(
        &ctx,
        &json!({
            "timezone": "Asia/Shanghai",
            "lang": "en_US.UTF-8",
            "aliases": {
                "claude": "TZ=\"{timezone}\" claude",
                "ll": "ls -l",
                "gs": "git status",
                "gp": "git push"
            }
        }),
    );
    // gs 已在区块里（同值）；claude 在区块里但取值不同
    write(
        &ctx,
        ".zshrc",
        &block(&["alias gs='git status'", "alias claude='claude --old'"]),
    );
    let db_before = std::fs::read(store::db_path(&ctx)).unwrap();
    let now = status(&ctx);
    assert_eq!(
        std::fs::read(store::db_path(&ctx)).unwrap(),
        db_before,
        "读路径零写"
    );
    let legacy = now["alias_legacy"].as_array().unwrap().clone();
    assert_eq!(names(&now["alias_legacy"]), ["claude", "gp", "ll"]);
    assert_eq!(legacy[0]["value"], "TZ=\"Asia/Shanghai\" claude");
    assert_eq!(legacy[0]["actual"], "alias claude='claude --old'");
    assert_eq!(legacy[1]["actual"], Value::Null);
    let id_of = |name: &str| {
        legacy.iter().find(|item| item["name"] == name).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    };

    // 加入 ll：只写 ~/.zshrc（~/.zprofile 没有受管区块，不新建）；写路径把已在区块里的 gs 与刚加入的 ll 从库里删掉
    let bases = both_bases(&now);
    let legacy_id = id_of("ll");
    let mut preview_args: Vec<&str> = bases.iter().map(String::as_str).collect();
    preview_args.extend_from_slice(&["--legacy", &legacy_id]);
    let preview = run(&ctx, "aliases:preview", &preview_args);
    assert!(preview.ok, "{:?}", preview.errors);
    assert!(!preview.lock.held);
    assert_eq!(
        read(&ctx, ".zshrc").unwrap(),
        block(&["alias gs='git status'", "alias claude='claude --old'"])
    );
    assert!(read(&ctx, ".zprofile").is_none());
    assert_eq!(std::fs::read(store::db_path(&ctx)).unwrap(), db_before);
    let env = apply(&ctx, &bases, &["--legacy", &legacy_id]);
    assert!(env.ok, "{:?}", env.errors);
    assert!(read(&ctx, ".zshrc").unwrap().contains("alias ll='ls -l'"));
    assert!(read(&ctx, ".zprofile").is_none());
    assert_eq!(
        read(&ctx, ".zshrc").as_deref(),
        preview.data["files"][0]["content"].as_str()
    );
    let after = status(&ctx);
    assert_eq!(names(&after["alias_legacy"]), ["claude", "gp"]);
    let stored: String = store::open_readonly(&ctx)
        .unwrap()
        .unwrap()
        .query_row(
            "SELECT value_json FROM alias_sources WHERE key = 'aliases.shell'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !stored.contains("ls -l") && !stored.contains("git status"),
        "{stored}"
    );

    // 丢弃 gp：只写库，rc 文件不变
    let zshrc_before = read(&ctx, ".zshrc").unwrap();
    let gp = after["alias_legacy"][1]["id"].as_str().unwrap().to_string();
    let env = run(&ctx, "aliases:discard-legacy", &["--legacy", &gp]);
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["source_removed"], false);
    assert_eq!(read(&ctx, ".zshrc").unwrap(), zshrc_before);
    let again = run(&ctx, "aliases:discard-legacy", &["--legacy", &gp]);
    assert!(!again.ok, "丢过的再丢要报已不存在");

    // 丢弃最后一条：整行删掉
    let claude = status(&ctx)["alias_legacy"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let env = run(&ctx, "aliases:discard-legacy", &["--legacy", &claude]);
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["source_removed"], true);
    assert_eq!(
        count(&ctx, "SELECT COUNT(*) FROM alias_sources"),
        0,
        "丢弃后库中无该来源"
    );
    assert_eq!(status(&ctx)["alias_legacy"], json!([]));
    let _ = std::fs::remove_dir_all(&root);
}

/// 升级场景：旧库留着默认来源行与旧 binding、rc 文件里是旧版本同步的区块。升级后直接读实际
/// 区块、没有建议、零写；旧 binding 不再是写入门槛（区块被外部改过也能按新 base 编辑）；
/// 旧表损坏只是不出建议，不挡住别名页，也不泄露内容。
#[test]
fn upgraded_database_reads_actual_blocks_and_old_tables_never_gate_writes() {
    let (root, ctx) = temp_ctx("upgrade");
    seed_legacy(
        &ctx,
        &json!({
            "timezone": "America/Los_Angeles",
            "lang": "en_US.UTF-8",
            "aliases": {
                "claude": "TZ=\"{timezone}\" LANG=\"{lang}\" LC_ALL=\"{lang}\" claude",
                "codex": "TZ=\"{timezone}\" LANG=\"{lang}\" LC_ALL=\"{lang}\" codex"
            }
        }),
    );
    {
        let conn = store::open_write(&ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE alias_target_bindings (
                target_id TEXT PRIMARY KEY NOT NULL,
                applied_digest TEXT NOT NULL,
                source_revision TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                applied_block_digest TEXT NOT NULL DEFAULT ''
            );
            INSERT INTO alias_target_bindings VALUES ('aliases:zshrc', 'stale', 'stale', 't', 'stale');",
        )
        .unwrap();
    }
    let synced = block(&[
        "alias claude='TZ=\"America/Los_Angeles\" LANG=\"en_US.UTF-8\" LC_ALL=\"en_US.UTF-8\" claude'",
        "alias codex='user-changed-codex'",
    ]);
    write(&ctx, ".zshrc", &format!("# mine\n\n{synced}"));
    add_alias_source(&ctx, "mine", &[("aliases.json", ALIAS_SOURCE)]);
    let db_before = std::fs::read(store::db_path(&ctx)).unwrap();
    let now = status(&ctx);
    assert_eq!(std::fs::read(store::db_path(&ctx)).unwrap(), db_before);
    assert_eq!(names(&now["aliases"]), ["claude", "codex"]);
    assert_eq!(now["aliases"][1]["value"], "user-changed-codex");
    assert_eq!(now["alias_legacy"], json!([]), "与内置相同的旧行不产生建议");
    assert_eq!(names(&now["alias_recommendations"]), ["chrome", "grok"]);

    let env = apply(
        &ctx,
        &["--zshrc-base".into(), base_of(&now, "aliases:zshrc")],
        &["--set", "codex", "--value", "codex --search"],
    );
    assert!(env.ok, "旧 binding 不得挡住写入: {:?}", env.errors);
    assert!(read(&ctx, ".zshrc")
        .unwrap()
        .contains("alias codex='codex --search'"));
    assert_eq!(
        count(&ctx, "SELECT COUNT(*) FROM alias_target_bindings"),
        1,
        "旧表原样留着"
    );

    // 旧来源行损坏：不出建议、不报错、不泄露
    let marker = "LEGACY-ROW-MARKER-8d21";
    {
        let conn = store::open_write(&ctx).unwrap();
        conn.execute(
            "UPDATE alias_sources SET value_json = ?1",
            [format!("{{\"aliases\":{{\"x\":\"bad'{marker}\"}}}}")],
        )
        .unwrap();
    }
    let env = run(&ctx, "aliases:status", &[]);
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["alias_legacy"], json!([]));
    assert!(!serde_json::to_string(&env).unwrap().contains(marker));
    let _ = std::fs::remove_dir_all(&root);
}

/// 别名来源已退役：sources:get/set 整个域不再存在，零写。
#[test]
fn sources_domain_is_retired() {
    let (root, ctx) = temp_ctx("sources-retired");
    for (command, args) in [
        ("sources:get", vec!["--key", "aliases"]),
        (
            "sources:set",
            vec!["--key", "aliases.shell", "--value", "{}"],
        ),
    ] {
        let env = run(&ctx, command, &args);
        assert!(!env.ok, "{command} 应被拒绝");
        assert_eq!(env.errors[0].code, "UNKNOWN_DOMAIN", "{command}");
    }
    assert!(!ctx.data_dir.exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// 写路径清理包含「写入前」已在区块里的旧版别名：删掉区块里那条后它不会作为建议冒出来。
#[test]
fn removing_an_alias_that_was_also_saved_does_not_resurrect_it() {
    let (root, ctx) = temp_ctx("no-resurrect");
    seed_legacy(&ctx, &json!({"aliases": {"ll": "ls -l"}}));
    write(&ctx, ".zshrc", &block(&["alias ll='ls -l'"]));
    let now = status(&ctx);
    assert_eq!(now["alias_legacy"], json!([]), "已在区块里的不重复提示");
    let env = apply(
        &ctx,
        &["--zshrc-base".into(), base_of(&now, "aliases:zshrc")],
        &["--remove", "ll"],
    );
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(status(&ctx)["alias_legacy"], json!([]));
    assert_eq!(count(&ctx, "SELECT COUNT(*) FROM alias_sources"), 0);
    let _ = std::fs::remove_dir_all(&root);
}
