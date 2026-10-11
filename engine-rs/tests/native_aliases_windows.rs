//! Windows 的启动命令：每条命令是 ~/.agent-manager/bin/<名>.cmd，查询、编辑、写前比对与 rc 文件
//! 同一套；本工具不改注册表，命令目录不在 PATH 上时给出让用户自己执行的命令。
#![cfg(windows)]

mod common;

use agent_manager_engine::platform::Canon;
use agent_manager_engine::{execute, Context, Envelope, Request};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

/// AGENT_MANAGER_USER_PATH 是进程级环境变量：改它的测试串行跑。
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    std::env::remove_var("AGENT_MANAGER_LOCK_HELD");
    std::env::remove_var("AGENT_MANAGER_LOCK_PID");
    let base = std::env::temp_dir().canon().unwrap().join(format!(
        "am-aliases-win-{}-{tag}-{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    common::private_fixture_root(&base);
    let ctx = Context {
        data_dir: base.join("data"),
        home_dir: base.join("home"),
    };
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
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

/// Generate and validate wrappers in the engine host that will execute them.
fn run_cli(ctx: &Context, command: &str, args: &[&str]) -> Value {
    let mut process = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"));
    process
        .args([command, "--json"])
        .args(args)
        .env_clear()
        .env("HOME", &ctx.home_dir)
        .env("USERPROFILE", &ctx.home_dir)
        .env("AGENT_MANAGER_DATA_DIR", &ctx.data_dir)
        .current_dir(&ctx.home_dir);
    windows_process_environment(&mut process);
    if let Some(path) = std::env::var_os("AGENT_MANAGER_USER_PATH") {
        process.env("AGENT_MANAGER_USER_PATH", path);
    }
    let output = process.output().unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "invalid engine JSON ({err}): {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(
        output.status.success(),
        result["ok"] == true,
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    result
}

fn windows_process_environment(process: &mut Command) {
    // Keep the prepared private temp root: Windows otherwise falls back to the
    // deliberately invalid USERPROFILE used by the account-isolation test.
    for name in [
        "PATH",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "TEMP",
        "TMP",
    ] {
        if let Some(value) = std::env::var_os(name) {
            process.env(name, value);
        }
    }
}

fn controlled_command(ctx: &Context, only: &str) -> String {
    let result = run_cli(ctx, "accounts:command", &["--only", only]);
    assert_eq!(result["ok"], true, "{}", result["errors"]);
    let value = result["data"]["launch_value"].as_str().unwrap().to_string();
    assert!(value.contains("--account-launch"));
    assert!(value.contains("--binding"));
    value
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

    let main_command = controlled_command(&ctx, "agents:claude-code");
    let value = format!("{main_command} --dangerously-skip-permissions");
    let applied = run_cli(
        &ctx,
        "aliases:apply",
        &["--cmd-base", &base(&ctx), "--set", "cc", "--value", &value],
    );
    assert_eq!(applied["ok"], true, "{}", applied["errors"]);
    let text = std::fs::read_to_string(bin_dir(&ctx).join("cc.cmd")).unwrap();
    assert!(
        text.contains("--only \"agents:claude-code\" --operation launch --binding "),
        "{text}"
    );
    assert!(
        text.ends_with(&format!("{} %*\r\n", value.replace('%', "%%"))),
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
    let preview = run_cli(
        &ctx,
        "aliases:preview",
        &[
            "--cmd-base",
            &base(&ctx),
            "--set",
            "cc",
            "--value",
            &main_command,
        ],
    );
    assert_eq!(preview["ok"], true, "{}", preview["errors"]);
    let files = preview["data"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0]["path"].as_str().unwrap().ends_with("cc.cmd"));
    assert_eq!(
        std::fs::read_to_string(bin_dir(&ctx).join("cc.cmd")).unwrap(),
        text,
        "预览不写"
    );

    // 改名：新名字新建、旧名字删除
    let renamed = run_cli(
        &ctx,
        "aliases:apply",
        &[
            "--cmd-base",
            &base(&ctx),
            "--set",
            "cw",
            "--value",
            &main_command,
            "--rename-from",
            "cc",
        ],
    );
    assert_eq!(renamed["ok"], true, "{}", renamed["errors"]);
    assert!(!bin_dir(&ctx).join("cc.cmd").exists());
    assert!(bin_dir(&ctx).join("cw.cmd").exists());

    // 外部改过：基准不对，拒绝，文件不变
    let stale = base(&ctx);
    std::fs::write(bin_dir(&ctx).join("cw.cmd"), "@echo off\r\nrem agent-manager launch command\r\nrem value: claude\r\nsetlocal\r\nclaude --x %*\r\n").unwrap();
    let rejected = run_cli(
        &ctx,
        "aliases:apply",
        &["--cmd-base", &stale, "--remove", "cw"],
    );
    assert_eq!(rejected["ok"], false, "{rejected}");
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
    let removed = run_cli(
        &ctx,
        "aliases:apply",
        &["--cmd-base", &base(&ctx), "--remove", "cw"],
    );
    assert_eq!(removed["ok"], true, "{}", removed["errors"]);
    assert!(!bin_dir(&ctx).join("cw.cmd").exists());

    // 与用户自己的脚本同名：不覆盖
    let clash = run_cli(
        &ctx,
        "aliases:apply",
        &[
            "--cmd-base",
            &base(&ctx),
            "--set",
            "mine",
            "--value",
            &main_command,
        ],
    );
    assert_eq!(clash["ok"], false, "{clash}");
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
fn 旧裸命令仍能读和删_新增裸助手命令拒绝且零写() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("AGENT_MANAGER_USER_PATH", r"C:\windows\system32");
    let (root, ctx) = temp_ctx("legacy-raw");
    std::fs::create_dir_all(bin_dir(&ctx)).unwrap();
    let legacy_value = r#"TZ="America/Los_Angeles" CLAUDE_CONFIG_DIR="$HOME/.claude-cc" claude --dangerously-skip-permissions"#;
    let legacy_text = format!(
        "@echo off\r\nrem agent-manager launch command\r\nrem value: {legacy_value}\r\nsetlocal\r\nset \"TZ=America/Los_Angeles\"\r\nset \"CLAUDE_CONFIG_DIR=%USERPROFILE%\\.claude-cc\"\r\nclaude --dangerously-skip-permissions %*\r\n"
    );
    let foreign_text = b"@echo off\r\necho mine\r\n";
    std::fs::write(bin_dir(&ctx).join("legacy.cmd"), &legacy_text).unwrap();
    std::fs::write(bin_dir(&ctx).join("mine.cmd"), foreign_text).unwrap();
    let status = run_cli(&ctx, "aliases:status", &[]);
    assert_eq!(status["ok"], true, "{}", status["errors"]);
    let legacy = status["data"]["aliases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|alias| alias["name"] == "legacy")
        .unwrap();
    assert_eq!(legacy["value"], legacy_value);
    assert_eq!(legacy["unsafe_account"], true);
    assert_eq!(legacy["targets"], json!(["aliases:cmd"]));

    for value in [
        "claude",
        "codex --model fixture-model",
        r#"CLAUDE_CONFIG_DIR="$HOME/.claude-other" claude"#,
        r#"CODEX_HOME="$HOME/.codex-other" codex"#,
    ] {
        for operation in ["aliases:preview", "aliases:apply"] {
            let result = run_cli(
                &ctx,
                operation,
                &[
                    "--cmd-base",
                    &base(&ctx),
                    "--set",
                    "unsafe",
                    "--value",
                    value,
                ],
            );
            assert_eq!(
                result["ok"], false,
                "{operation} unexpectedly accepted {value}"
            );
            assert!(
                result["errors"].to_string().contains("账号预检"),
                "{result}"
            );
            assert_eq!(
                std::fs::read(bin_dir(&ctx).join("legacy.cmd")).unwrap(),
                legacy_text.as_bytes()
            );
            assert_eq!(
                std::fs::read(bin_dir(&ctx).join("mine.cmd")).unwrap(),
                foreign_text
            );
            assert!(!bin_dir(&ctx).join("unsafe.cmd").exists());
            assert!(
                !ctx.data_dir.exists(),
                "unsafe command preflight must not write the database"
            );
        }
    }
    let removed = run_cli(
        &ctx,
        "aliases:apply",
        &["--cmd-base", &base(&ctx), "--remove", "legacy"],
    );
    assert_eq!(removed["ok"], true, "{}", removed["errors"]);
    assert!(!bin_dir(&ctx).join("legacy.cmd").exists());
    assert_eq!(
        std::fs::read(bin_dir(&ctx).join("mine.cmd")).unwrap(),
        foreign_text
    );
    std::env::remove_var("AGENT_MANAGER_USER_PATH");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn 生成的脚本在_cmd_里真能跑_环境变量与参数都传到程序() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("AGENT_MANAGER_USER_PATH", r"C:\windows\system32");
    let (root, ctx) = temp_ctx("run");
    // Native fake CLI records the context received through the registered wrapper.
    common::install_fake_account_clis(&ctx.home_dir);
    let definition = json!({
        "bin": "claude",
        "profile": {
            "base_agent": "claude-code",
            "config_dir": "~/.claude-cc",
            "env": {"TZ": "America/Los_Angeles"}
        }
    })
    .to_string();
    let created = run_cli(
        &ctx,
        "profiles:create",
        &[
            "--only",
            "profiles:cc",
            "--base",
            "missing",
            "--definition",
            &definition,
        ],
    );
    assert_eq!(created["ok"], true, "{}", created["errors"]);
    let value = format!(
        "{} --model fixture-model",
        controlled_command(&ctx, "profiles:cc")
    );
    let applied = run_cli(
        &ctx,
        "aliases:apply",
        &["--cmd-base", "missing", "--set", "cc", "--value", &value],
    );
    assert_eq!(applied["ok"], true, "{}", applied["errors"]);

    let mut process = Command::new(std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into()));
    process
        .args(["/d", "/c", "cc", "extra arg"])
        .current_dir(bin_dir(&ctx))
        .env_clear()
        .env("HOME", root.join("wrong-home"))
        .env("USERPROFILE", root.join("wrong-home"))
        .env("CLAUDE_CONFIG_DIR", root.join("wrong-account"))
        .env("ANTHROPIC_AUTH_TOKEN", "parent-dummy-token")
        .env("ANTHROPIC_BASE_URL", "https://parent.example.invalid")
        .env("TZ", "parent-timezone");
    windows_process_environment(&mut process);
    let out = process.output().unwrap();
    assert!(
        out.status.success(),
        "{} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let environment: Value =
        serde_json::from_slice(&std::fs::read(ctx.home_dir.join(".fake-cli-env")).unwrap())
            .unwrap();
    let arguments: Value =
        serde_json::from_slice(&std::fs::read(ctx.home_dir.join(".fake-cli-argv")).unwrap())
            .unwrap();
    assert_eq!(
        environment["CLAUDE_CONFIG_DIR"],
        ctx.home_dir
            .join(".claude-cc")
            .canon()
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(environment["HOME"], ctx.home_dir.to_string_lossy().as_ref());
    assert_eq!(
        environment["USERPROFILE"],
        ctx.home_dir.to_string_lossy().as_ref()
    );
    assert_eq!(environment["TZ"], "America/Los_Angeles");
    assert_eq!(environment["ANTHROPIC_AUTH_TOKEN"], "unset");
    assert_eq!(environment["ANTHROPIC_BASE_URL"], "unset");
    assert_eq!(arguments, json!(["--model", "fixture-model", "extra arg"]));
    std::env::remove_var("AGENT_MANAGER_USER_PATH");
    let _ = std::fs::remove_dir_all(root);
}
