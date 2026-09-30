//! bash 用户（多数 Linux 发行版的默认 shell）：启动命令写 ~/.bashrc；~/.zshrc 已有本工具那一段时同步，
//! 没有就不新建。单独一个测试进程，设 SHELL=/bin/bash 不影响其他用例。
#![cfg(unix)]

use agent_manager_engine::{execute, Context, Envelope, Request};
use serde_json::Value;
use std::path::PathBuf;

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    std::env::set_var("SHELL", "/bin/bash");
    std::env::remove_var("AGENT_MANAGER_LOCK_HELD");
    std::env::remove_var("AGENT_MANAGER_LOCK_PID");
    let base = std::env::temp_dir().join(format!(
        "am-aliases-bash-{}-{tag}-{:x}",
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
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    (base, ctx)
}

fn run(ctx: &Context, command: &str, args: &[&str]) -> Envelope {
    execute(
        ctx,
        &Request {
            command: command.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            run_id: "aliases-bash-test".to_string(),
        },
    )
}

/// 各目标的 (写入基准参数, 基准)，与界面一样全部带上。
fn bases(ctx: &Context) -> (Vec<String>, Value) {
    let env = run(ctx, "aliases:status", &[]);
    assert!(env.ok, "{:?}", env.errors);
    let mut out = Vec::new();
    for element in env.data["elements"].as_array().unwrap() {
        if let Some(base) = element["observed"]["base"].as_str() {
            out.push(element["spec"]["base_flag"].as_str().unwrap().to_string());
            out.push(base.to_string());
        }
    }
    (out, env.data)
}

#[test]
fn bash_用户新命令写进_bashrc_zshrc_没有那一段就不新建() {
    let (root, ctx) = temp_ctx("primary");
    std::fs::write(
        ctx.home_dir.join(".bashrc"),
        "# 用户自己的\nexport EDITOR=vim\n",
    )
    .unwrap();
    let (flags, status) = bases(&ctx);
    assert_eq!(
        status["elements"][0]["id"], "aliases:bashrc",
        "bash 用户以 ~/.bashrc 为主"
    );

    let mut args: Vec<&str> = flags.iter().map(String::as_str).collect();
    args.extend(["--set", "cc", "--value", "claude --flag"]);
    let applied = run(&ctx, "aliases:apply", &args);
    assert!(applied.ok, "{:?}", applied.errors);
    let bashrc = std::fs::read_to_string(ctx.home_dir.join(".bashrc")).unwrap();
    assert!(
        bashrc.starts_with("# 用户自己的\nexport EDITOR=vim\n"),
        "原有内容不动：{bashrc}"
    );
    assert!(bashrc.contains("alias cc='claude --flag'"), "{bashrc}");
    assert!(
        !ctx.home_dir.join(".zshrc").exists(),
        "没有受管区块的 ~/.zshrc 不新建"
    );

    // 真让 bash 读一遍：交互式 bash 里能用到这条命令
    let out = std::process::Command::new("bash")
        .args(["-ic", "alias cc"])
        .env("HOME", &ctx.home_dir)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("claude --flag"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // ~/.zshrc 已有本工具那一段：同步改它
    std::fs::write(
        ctx.home_dir.join(".zshrc"),
        "# >>> agent-manager shell aliases >>>\nalias cc='claude --flag'\n# <<< agent-manager shell aliases <<<\n",
    )
    .unwrap();
    let (flags, _) = bases(&ctx);
    let mut args: Vec<&str> = flags.iter().map(String::as_str).collect();
    args.extend(["--set", "cc", "--value", "claude --other"]);
    let applied = run(&ctx, "aliases:apply", &args);
    assert!(applied.ok, "{:?}", applied.errors);
    for file in [".bashrc", ".zshrc"] {
        let text = std::fs::read_to_string(ctx.home_dir.join(file)).unwrap();
        assert!(text.contains("alias cc='claude --other'"), "{file}: {text}");
    }
    let _ = std::fs::remove_dir_all(root);
}
