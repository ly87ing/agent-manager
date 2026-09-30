//! Windows 的启动命令：每条命令是 ~/.agent-manager/bin/<名>.cmd，查询、编辑、写前比对与 rc 文件
//! 同一套；本工具不改注册表，命令目录不在 PATH 上时给出让用户自己执行的命令。
#![cfg(windows)]

use agent_manager_engine::{execute, Context, Envelope, Request};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;

/// AGENT_MANAGER_USER_PATH 是进程级环境变量：改它的测试串行跑。
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    std::env::remove_var("AGENT_MANAGER_LOCK_HELD");
    std::env::remove_var("AGENT_MANAGER_LOCK_PID");
    let base = std::env::temp_dir().join(format!(
        "am-aliases-win-{}-{tag}-{:x}",
        std::process::id(),
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

fn run(ctx: &Context, command: &str, args: &[&str]) -> Envelope {
    execute(
        ctx,
        &Request {
            command: command.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            run_id: "aliases-win-test".to_string(),
        },
    )
}

fn target(ctx: &Context) -> Value {
    let env = run(ctx, "aliases:status", &[]);
    assert!(env.ok, "{:?}", env.errors);
    let elements = env.data["elements"].as_array().unwrap().clone();
    assert_eq!(elements.len(), 1, "Windows 只有命令脚本目录这一个目标");
    assert_eq!(elements[0]["id"], "aliases:cmd");
    assert_eq!(elements[0]["spec"]["kind"], "cmd-dir");
    elements[0].clone()
}

fn base(ctx: &Context) -> String {
    target(ctx)["observed"]["base"]
        .as_str()
        .unwrap()
        .to_string()
}

fn bin_dir(ctx: &Context) -> PathBuf {
    ctx.home_dir.join(".agent-manager").join("bin")
}

#[test]
fn 新建_改写_删除都落在命令脚本上_用户自己的脚本不动() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("AGENT_MANAGER_USER_PATH", r"C:\windows\system32");
    let (root, ctx) = temp_ctx("crud");
    // 没有目录：可以新建，基准为 missing，查询不建任何东西
    assert_eq!(base(&ctx), "missing");
    assert!(!bin_dir(&ctx).exists(), "查询零写");

    // 用户自己的脚本：只读列出
    std::fs::create_dir_all(bin_dir(&ctx)).unwrap();
    std::fs::write(bin_dir(&ctx).join("mine.cmd"), "@echo off\r\necho mine\r\n").unwrap();
    let t = target(&ctx);
    assert_eq!(t["observed"]["external"][0]["name"], "mine");

    let value = r#"CLAUDE_CONFIG_DIR="$HOME/.claude-cc" claude --dangerously-skip-permissions"#;
    let applied = run(
        &ctx,
        "aliases:apply",
        &["--cmd-base", &base(&ctx), "--set", "cc", "--value", value],
    );
    assert!(applied.ok, "{:?}", applied.errors);
    let text = std::fs::read_to_string(bin_dir(&ctx).join("cc.cmd")).unwrap();
    assert!(
        text.contains("set \"CLAUDE_CONFIG_DIR=%USERPROFILE%\\.claude-cc\"\r\n"),
        "{text}"
    );
    assert!(
        text.ends_with("claude --dangerously-skip-permissions %*\r\n"),
        "{text}"
    );

    // 读回：取值与写入的一致，合并视图里也有
    let env = run(&ctx, "aliases:status", &[]);
    let cc = env.data["aliases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "cc")
        .unwrap()
        .clone();
    assert_eq!(cc["value"], value);
    assert_eq!(cc["targets"], serde_json::json!(["aliases:cmd"]));

    // 预览：只列要改的脚本，不写
    let preview = run(
        &ctx,
        "aliases:preview",
        &[
            "--cmd-base",
            &base(&ctx),
            "--set",
            "cc",
            "--value",
            "claude",
        ],
    );
    assert!(preview.ok, "{:?}", preview.errors);
    let files = preview.data["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0]["path"].as_str().unwrap().ends_with("cc.cmd"));
    assert_eq!(
        std::fs::read_to_string(bin_dir(&ctx).join("cc.cmd")).unwrap(),
        text,
        "预览不写"
    );

    // 改名：新名字新建、旧名字删除
    let renamed = run(
        &ctx,
        "aliases:apply",
        &[
            "--cmd-base",
            &base(&ctx),
            "--set",
            "cw",
            "--value",
            "claude",
            "--rename-from",
            "cc",
        ],
    );
    assert!(renamed.ok, "{:?}", renamed.errors);
    assert!(!bin_dir(&ctx).join("cc.cmd").exists());
    assert!(bin_dir(&ctx).join("cw.cmd").exists());

    // 外部改过：基准不对，拒绝，文件不变
    let stale = base(&ctx);
    std::fs::write(bin_dir(&ctx).join("cw.cmd"), "@echo off\r\nrem agent-manager launch command\r\nrem value: claude\r\nsetlocal\r\nclaude --x %*\r\n").unwrap();
    let rejected = run(
        &ctx,
        "aliases:apply",
        &["--cmd-base", &stale, "--remove", "cw"],
    );
    assert!(!rejected.ok);
    assert!(bin_dir(&ctx).join("cw.cmd").exists());
    // 手改过的只能删除：取值认不出
    let env = run(&ctx, "aliases:status", &[]);
    let cw = env.data["aliases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "cw")
        .unwrap()
        .clone();
    assert_eq!(cw["value"], Value::Null);
    let removed = run(
        &ctx,
        "aliases:apply",
        &["--cmd-base", &base(&ctx), "--remove", "cw"],
    );
    assert!(removed.ok, "{:?}", removed.errors);
    assert!(!bin_dir(&ctx).join("cw.cmd").exists());

    // 与用户自己的脚本同名：不覆盖
    let clash = run(
        &ctx,
        "aliases:apply",
        &[
            "--cmd-base",
            &base(&ctx),
            "--set",
            "mine",
            "--value",
            "claude",
        ],
    );
    assert!(!clash.ok);
    assert_eq!(
        std::fs::read_to_string(bin_dir(&ctx).join("mine.cmd")).unwrap(),
        "@echo off\r\necho mine\r\n"
    );
    std::env::remove_var("AGENT_MANAGER_USER_PATH");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn 命令目录不在_path_上时给出自己执行的命令_在时不给() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (root, ctx) = temp_ctx("path");
    std::env::set_var("AGENT_MANAGER_USER_PATH", r"C:\windows\system32");
    let t = target(&ctx);
    assert_eq!(t["observed"]["on_path"], false);
    let command = t["observed"]["path_command"].as_str().unwrap();
    assert!(
        command.contains(&bin_dir(&ctx).display().to_string()),
        "{command}"
    );
    assert!(
        command.contains("DoNotExpandEnvironmentNames"),
        "按原始值读写，不展开已有的 %VAR%"
    );
    assert!(t["observed"]["path_manual"]
        .as_str()
        .unwrap()
        .contains("EditEnvironmentVariables"));

    std::env::set_var(
        "AGENT_MANAGER_USER_PATH",
        format!(r"C:\windows\system32;{}", bin_dir(&ctx).display()),
    );
    let t = target(&ctx);
    assert_eq!(t["observed"]["on_path"], true);
    assert_eq!(t["observed"]["path_command"], Value::Null);
    std::env::remove_var("AGENT_MANAGER_USER_PATH");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn 生成的脚本在_cmd_里真能跑_环境变量与参数都传到程序() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("AGENT_MANAGER_USER_PATH", r"C:\windows\system32");
    let (root, ctx) = temp_ctx("run");
    // 冒充 claude：打印拿到的配置目录、时区与参数
    let fake = root.join("fakebin");
    std::fs::create_dir_all(&fake).unwrap();
    std::fs::write(
        fake.join("claude.cmd"),
        "@echo off\r\necho DIR=%CLAUDE_CONFIG_DIR%\r\necho TZ=%TZ%\r\necho ARGS=%*\r\n",
    )
    .unwrap();
    let value = r#"TZ="America/Los_Angeles" CLAUDE_CONFIG_DIR="$HOME/.claude-cc" claude --flag"#;
    let applied = run(
        &ctx,
        "aliases:apply",
        &["--cmd-base", "missing", "--set", "cc", "--value", value],
    );
    assert!(applied.ok, "{:?}", applied.errors);

    let path = format!(
        "{};{}",
        fake.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = std::process::Command::new("cmd")
        .args(["/c", "cc", "extra arg"])
        .current_dir(bin_dir(&ctx))
        .env("PATH", path)
        .env("USERPROFILE", &ctx.home_dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!("DIR={}\\.claude-cc", ctx.home_dir.display())),
        "{stdout}"
    );
    assert!(stdout.contains("TZ=America/Los_Angeles"), "{stdout}");
    assert!(stdout.contains("ARGS=--flag \"extra arg\""), "{stdout}");
    std::env::remove_var("AGENT_MANAGER_USER_PATH");
    let _ = std::fs::remove_dir_all(root);
}
