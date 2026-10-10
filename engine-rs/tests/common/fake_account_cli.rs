//! Standalone std-only fixture: compiled by tests, never shipped with the engine.
use std::io::Write;
use std::path::{Path, PathBuf};

fn append(path: &Path, text: &str) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "{text}").unwrap();
}
fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| "unset".into())
}
fn json_string(value: &str) -> String {
    let mut result = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            ch if ch.is_control() => result.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => result.push(ch),
        }
    }
    result.push('"');
    result
}
fn main() {
    let executable = std::env::current_exe().unwrap();
    let codex = executable
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .eq_ignore_ascii_case("codex");
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--version") {
        println!(
            "{}",
            if codex {
                "codex-cli 0.156.1"
            } else {
                "2.1.285 (Claude Code)"
            }
        );
        return;
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            executable
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf()
        });
    let (settings, command) = if args.first().map(String::as_str) == Some("--settings") {
        (
            args.get(1).map(String::as_str).unwrap_or("none"),
            &args[args.len().min(2)..],
        )
    } else {
        ("none", args.as_slice())
    };
    let status = command.first().map(String::as_str) == Some(if codex { "login" } else { "auth" })
        && command.get(1).map(String::as_str) == Some("status");
    if !status {
        if matches!(
            command.first().map(String::as_str),
            Some("auth" | "login" | "logout")
        ) {
            std::process::exit(64);
        }
        // The aliases fixture reads the controlled child's actual input.
        let argv = args
            .iter()
            .map(|arg| json_string(arg))
            .collect::<Vec<_>>()
            .join(",");
        std::fs::write(home.join(".fake-cli-argv"), format!("[{argv}]\n")).unwrap();
        let fields = [
            "CLAUDE_CONFIG_DIR",
            "CODEX_HOME",
            "TZ",
            "HOME",
            "USERPROFILE",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
        ]
        .iter()
        .map(|name| format!("{}:{}", json_string(name), json_string(&env(name))))
        .collect::<Vec<_>>()
        .join(",");
        std::fs::write(home.join(".fake-cli-env"), format!("{{{fields}}}\n")).unwrap();
        return;
    }
    let directory = PathBuf::from(
        std::env::var_os(if codex {
            "CODEX_HOME"
        } else {
            "CLAUDE_CONFIG_DIR"
        })
        .unwrap_or_default(),
    );
    append(
        &home.join(".probe-log"),
        &format!(
            "{}|{}|{}",
            if codex { "codex" } else { "claude" },
            directory.display(),
            env(if codex {
                "CODEX_API_KEY"
            } else {
                "CLAUDE_CODE_OAUTH_TOKEN"
            })
        ),
    );
    if codex {
        if directory.join(".fake-logged-in").exists() {
            eprintln!("Logged in using ChatGPT");
        } else {
            eprintln!("Not logged in");
            std::process::exit(1);
        }
        return;
    }
    append(
        &home.join(".probe-args"),
        &format!(
            "{settings}|{}|{}",
            env("ANTHROPIC_CONFIG_DIR"),
            env("ANTHROPIC_PROFILE")
        ),
    );
    if home.join(".fake-garbage").exists() {
        println!("not json");
    } else if directory.join(".fake-logged-in").exists() {
        println!(r#"{{"loggedIn":true,"authMethod":"claude.ai"}}"#);
    } else if directory.join(".fake-console").exists() {
        println!(
            r#"{{"loggedIn":true,"authMethod":"oauth_token","apiKeySource":"/login managed key"}}"#
        );
    } else if directory.join(".fake-helper").exists() {
        println!(
            r#"{{"loggedIn":true,"authMethod":"api_key_helper","apiKeySource":"apiKeyHelper"}}"#
        );
    } else {
        println!(r#"{{"loggedIn":false,"authMethod":"none"}}"#);
        std::process::exit(1);
    }
}
