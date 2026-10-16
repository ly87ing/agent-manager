use agent_manager_engine::{execute, Context, Request};
use std::path::PathBuf;

/// 找不到可用的主目录就退出：退回启动目录会把配置和数据写到不该写的地方。
fn home_dir() -> PathBuf {
    agent_manager_engine::platform::home_dir().unwrap_or_else(|| {
        eprintln!("找不到用户主目录：HOME（Windows 上为 USERPROFILE）没有设置或不是绝对路径");
        std::process::exit(2);
    })
}
fn data_dir() -> PathBuf {
    agent_manager_engine::data_dir::resolve(Some(agent_manager_engine::data_dir::preferred(
        &home_dir(),
    )))
    .expect("The home directory supplies a default data directory")
}
fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = agent_manager_engine::environment_clock_helper_cli(&argv) {
        std::process::exit(code);
    }
    if argv.first().map(String::as_str) == Some("--account-launch") {
        match agent_manager_engine::native_account_context::run_cli(&argv[1..]) {
            Ok(code) => std::process::exit(code),
            Err(err) => {
                eprintln!(
                    "{}",
                    agent_manager_engine::sanitize::sanitize_diagnostic_message(&format!(
                        "{err:#}"
                    ))
                );
                std::process::exit(2);
            }
        }
    }
    let mut command = None;
    let mut args = Vec::new();
    let mut json = false;
    for item in argv {
        if item == "--json" {
            json = true;
        } else if command.is_none() && !item.starts_with('-') {
            command = Some(item);
        } else {
            args.push(item);
        }
    }
    let command = command.unwrap_or_default();
    if !json {
        eprintln!("必须显式追加 --json");
        std::process::exit(2);
    }
    let request = Request {
        command,
        args,
        run_id: std::env::var("AGENT_MANAGER_RUN_ID")
            .unwrap_or_else(|_| format!("cli-{}", std::process::id())),
    };
    let ctx = Context {
        data_dir: data_dir(),
        home_dir: home_dir(),
    };
    let envelope = execute(&ctx, &request);
    let failed = !envelope.ok;
    envelope.emit();
    std::process::exit(if failed { 1 } else { 0 });
}
