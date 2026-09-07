//! agent-manager-engine — Engine Contract v1 CLI (Rust slice).
//!
//! Dual implementation of src/engine_v1.py (Python). This slice implements the
//! mcp domain (status + plan); every other contract command answers
//! DOMAIN_NOT_IMPLEMENTED exactly like the Python engine did before its slice
//! landed. Exit codes: 0 = ok (warnings allowed), 1 = errors non-empty,
//! 2 = contract/usage error.

mod agents;
mod aliases;
mod canon;
mod envelope;
mod ids;
mod mcp;
mod overlay;
mod hooks;
mod providers;
mod rules;
mod skills;
mod upstream;

use envelope::{diagnostic, Envelope, Lock, Mode};
use std::path::PathBuf;

const KNOWN_DOMAINS: &[&str] = &[
    "agents", "mcp", "skills", "rules", "hooks", "tools", "aliases", "secrets", "profile", "policy",
    "system", "runs", "snapshots",
];

const VALUE_FLAGS: &[&str] = &[
    "--config", "--def-file", "--python-cmd", "--only", "--agent", "--run", "--lines", "--layer", "--key",
    "--value",
];
const BOOL_FLAGS: &[&str] = &["--json", "--dry-run", "--offline", "--allow-uninstall"];

struct Args {
    command: Option<String>,
    config: Option<String>,
    def_files: Vec<String>,
    only: Vec<String>,
    agent: Option<String>,
    json: bool,
    dry_run: bool,
    offline: bool,
    leftover: Vec<String>,
}

fn parse_args(argv: &[String]) -> Result<Args, ()> {
    let mut args = Args {
        command: None,
        config: None,
        def_files: Vec::new(),
        only: Vec::new(),
        agent: None,
        json: false,
        dry_run: false,
        offline: false,
        leftover: Vec::new(),
    };
    let mut index = 0;
    while index < argv.len() {
        let token = &argv[index];
        if BOOL_FLAGS.contains(&token.as_str()) {
            match token.as_str() {
                "--json" => args.json = true,
                "--dry-run" => args.dry_run = true,
                "--offline" => args.offline = true,
                _ => {}
            }
            index += 1;
            continue;
        }
        if VALUE_FLAGS.contains(&token.as_str()) {
            let value = match argv.get(index + 1) {
                Some(value) if !value.starts_with("--") => value.clone(),
                _ => return Err(()),
            };
            match token.as_str() {
                "--config" => args.config = Some(value),
                "--def-file" => args.def_files.push(value),
                "--only" => args.only.push(value),
                "--agent" => args.agent = Some(value),
                _ => {} // accepted for argv parity with the Python engine; unused here
            }
            index += 2;
            continue;
        }
        if token.starts_with("--") {
            args.leftover.push(token.clone());
        } else if args.command.is_none() {
            args.command = Some(token.clone());
        } else {
            args.leftover.push(token.clone());
        }
        index += 1;
    }
    Ok(args)
}

fn generate_run_id() -> String {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    format!("{}-{:x}", stamp, std::process::id())
}

struct RunContext {
    run_id: String,
    log_file: Option<String>,
    lock: Lock,
    mode: Mode,
}

fn run_context(args: &Args) -> RunContext {
    let run_id = std::env::var("AGENT_MANAGER_RUN_ID")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(generate_run_id);
    let log_file = std::env::var("AGENT_MANAGER_LOG_FILE")
        .ok()
        .filter(|value| !value.is_empty());
    let held = std::env::var("AGENT_MANAGER_LOCK_HELD").ok().as_deref() == Some("true");
    let lock = if held {
        let pid = std::env::var("AGENT_MANAGER_LOCK_PID")
            .ok()
            .and_then(|raw| raw.parse::<i64>().ok())
            .unwrap_or_else(|| std::process::id() as i64);
        Lock { held: true, pid: Some(pid) }
    } else {
        Lock { held: false, pid: None }
    };
    RunContext {
        run_id,
        log_file,
        lock,
        mode: Mode { dry_run: args.dry_run, offline: args.offline },
    }
}

fn usage_error(ctx: &RunContext, command: &str, code: &str, message: &str, hint: &str) -> i32 {
    Envelope::new(
        if command.is_empty() { "unknown".to_string() } else { command.to_string() },
        Mode { dry_run: ctx.mode.dry_run, offline: ctx.mode.offline },
        Lock { held: ctx.lock.held, pid: ctx.lock.pid },
        ctx.run_id.clone(),
        ctx.log_file.clone(),
        serde_json::Value::Object(serde_json::Map::new()),
        vec![],
        vec![diagnostic(code, None, message.to_string())],
    )
    .emit();
    eprintln!("{}", hint);
    2
}

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(args) => args,
        Err(()) => {
            let ctx = run_context(&Args {
                command: None,
                config: None,
                def_files: vec![],
                only: vec![],
                agent: None,
                json: false,
                dry_run: false,
                offline: false,
                leftover: vec![],
            });
            return usage_error(
                &ctx,
                "unknown",
                "USAGE_ERROR",
                "参数解析失败；期望 <domain>:<verb> --config <path> --json",
                "错误: 参数解析失败（值参数缺值）",
            );
        }
    };
    let ctx = run_context(&args);
    let raw_command = args.command.clone().unwrap_or_default();
    let dotted = raw_command.replacen(':', ".", 1);

    if raw_command.is_empty() {
        return usage_error(
            &ctx,
            &dotted,
            "USAGE_ERROR",
            "缺少命令；期望 <domain>:status 形式（如 mcp:status）",
            "用法: agent-manager-engine <domain>:<verb> --config <merged-config> [--def-file <path>] --json [--dry-run] [--offline]",
        );
    }
    if !args.json {
        return usage_error(
            &ctx,
            &dotted,
            "JSON_REQUIRED",
            &format!("命令 '{}' 是机器接口，必须显式追加 --json", dotted),
            "错误: 引擎命令只输出 envelope JSON；请追加 --json（人类可读渲染由 GUI/渲染层负责）",
        );
    }
    if !args.leftover.is_empty() {
        return usage_error(
            &ctx,
            &dotted,
            "USAGE_ERROR",
            &format!("无法识别的参数: {}", args.leftover.join(" ")),
            &format!("错误: 无法识别的参数 '{}'", args.leftover.join(" ")),
        );
    }
    if args.config.is_none() {
        return usage_error(
            &ctx,
            &dotted,
            "USAGE_ERROR",
            "缺少 --config <merged-config>（三层合并后的策略文件）",
            "错误: 缺少 --config 参数",
        );
    }

    let mut parts = dotted.splitn(2, '.');
    let domain = parts.next().unwrap_or("");
    let verb = parts.next().unwrap_or("");
    if domain.is_empty() || verb.is_empty() {
        return usage_error(
            &ctx,
            &dotted,
            "USAGE_ERROR",
            &format!("命令 '{}' 不是 <domain>:<verb> 形式", raw_command),
            "错误: 命令须为 <domain>:<verb> 形式（如 mcp:status）",
        );
    }
    if !KNOWN_DOMAINS.contains(&domain) {
        return usage_error(
            &ctx,
            &dotted,
            "UNKNOWN_DOMAIN",
            &format!("未知域 '{}'（已知域: {}）", domain, KNOWN_DOMAINS.join(", ")),
            &format!("错误: 未知域 '{}'；当前已接线域: mcp", domain),
        );
    }

    let implemented = (domain == "mcp" || domain == "agents" || domain == "skills" || domain == "rules" || domain == "hooks" || domain == "aliases") && (verb == "status" || verb == "plan");
    if !implemented {
        Envelope::new(
            dotted.clone(),
            Mode { dry_run: ctx.mode.dry_run, offline: ctx.mode.offline },
            Lock { held: ctx.lock.held, pid: ctx.lock.pid },
            ctx.run_id.clone(),
            ctx.log_file.clone(),
            serde_json::Value::Object(serde_json::Map::new()),
            vec![],
            vec![diagnostic(
                "DOMAIN_NOT_IMPLEMENTED",
                None,
                format!("命令 '{}' 属于契约命令面，但在本切片尚未实现", dotted),
            )],
        )
        .emit();
        eprintln!(
            "提示: '{}' 尚未实现（后续切片落地）；Rust 切片当前可用: mcp/agents/skills/rules/hooks/aliases 的 status/plan",
            dotted
        );
        return 2;
    }

    if args.def_files.is_empty() {
        return usage_error(
            &ctx,
            &dotted,
            "USAGE_ERROR",
            &format!("{} 需要 --def-file（域定义文件）", dotted),
            "错误: 缺少 --def-file 参数",
        );
    }

    // Python 引擎的 root 由脚本位置推导（Path(__file__).resolve()，会解析符号
    // 链接）；共享二进制由 manage.sh 经 AGENT_MANAGER_ROOT_DIR 注入，须同样解析。
    let root_dir = std::env::var("AGENT_MANAGER_ROOT_DIR")
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("engine-rs lives at <repo>/engine-rs")
                .to_path_buf()
        });
    let root_dir = std::fs::canonicalize(&root_dir).unwrap_or(root_dir);
    let config_file = PathBuf::from(args.config.clone().unwrap());
    let config_file = if config_file.is_absolute() {
        config_file
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(config_file)
    };
    let def_file = PathBuf::from(args.def_files[0].clone());
    let def_file = if def_file.is_absolute() {
        def_file
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(def_file)
    };
    let def_files: Vec<PathBuf> = args
        .def_files
        .iter()
        .map(|raw| {
            let path = PathBuf::from(raw);
            if path.is_absolute() {
                path
            } else {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(path)
            }
        })
        .collect();

    // 跨域元素与非法范围预检
    let domain_prefix = match domain {
        "mcp" => "mcp:",
        "agents" => "agents:",
        "skills" => "skill:",
        "rules" => "rules:",
        "hooks" => "hooks:",
        "aliases" => "aliases:",
        _ => "",
    };
    for item in &args.only {
        if !domain_prefix.is_empty() && !item.starts_with(domain_prefix) {
            return usage_error(
                &ctx,
                &dotted,
                "CROSS_DOMAIN_SCOPE",
                &format!("跨域元素 '{}' 不属于域 '{}'", item, domain),
                &format!("错误: 跨域元素 '{}' 不属于域 '{}'", item, domain),
            );
        }
    }
    if args.agent.is_some() && domain != "skills" {
        return usage_error(
            &ctx,
            &dotted,
            "USAGE_ERROR",
            "--agent 参数仅适用于 skills 域",
            "错误: --agent 参数仅适用于 skills 域",
        );
    }

    let result = match (domain, verb) {
        ("mcp", "status") => mcp::build_mcp_status(&root_dir, &config_file, &def_file),
        ("mcp", "plan") => mcp::build_mcp_plan(&root_dir, &config_file, &def_file, &args.only),
        ("agents", "status") => agents::build_agents_status(&root_dir, &config_file, &def_files),
        ("agents", "plan") => agents::build_agents_plan(&root_dir, &config_file, &def_files, &args.only),
        ("skills", "status") => skills::build_skills_status(&root_dir, &config_file, &def_files),
        ("skills", "plan") => skills::build_skills_plan(&root_dir, &config_file, &def_files, &args.only, args.agent.as_deref()),
        ("rules", "status") => rules::build_rules_status(&root_dir, &config_file, &def_files),
        ("rules", "plan") => rules::build_rules_plan(&root_dir, &config_file, &def_files, &args.only),
        ("hooks", "status") => hooks::build_hooks_status(&root_dir, &config_file, &def_files),
        ("hooks", "plan") => hooks::build_hooks_plan(&root_dir, &config_file, &def_files, &args.only),
        ("aliases", "status") => aliases::build_aliases_status(&root_dir, &config_file, &def_files),
        ("aliases", "plan") => aliases::build_aliases_plan(&root_dir, &config_file, &def_files, &args.only),
        _ => unreachable!("dispatch guard above"),
    };
    match result {
        Ok((data, warnings, errors)) => {
            let error_count = errors.len();
            Envelope::new(
                dotted.clone(),
                Mode { dry_run: ctx.mode.dry_run, offline: ctx.mode.offline },
                Lock { held: ctx.lock.held, pid: ctx.lock.pid },
                ctx.run_id.clone(),
                ctx.log_file.clone(),
                data,
                warnings.clone(),
                errors,
            )
            .emit();
            for warning in &warnings {
                eprintln!("警告: [{}] {}", warning.code, warning.message);
            }
            if error_count > 0 {
                eprintln!("错误: {} 完成但含 {} 个错误", dotted, error_count);
                1
            } else {
                0
            }
        }
        Err(exc) => {
            Envelope::new(
                dotted.clone(),
                Mode { dry_run: ctx.mode.dry_run, offline: ctx.mode.offline },
                Lock { held: ctx.lock.held, pid: ctx.lock.pid },
                ctx.run_id.clone(),
                ctx.log_file.clone(),
                serde_json::Value::Object(serde_json::Map::new()),
                vec![],
                vec![diagnostic("CONFIG_INVALID", None, exc.to_string())],
            )
            .emit();
            eprintln!("错误: {}", exc);
            1
        }
    }
}
