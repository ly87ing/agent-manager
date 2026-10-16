//! Real CLI and native stdio protocol acceptance, including Windows batch shims.
#[cfg(windows)]
mod common;

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TOKEN: &str = "YOUR_FAKE_SELECTED_MCP_TOKEN_42";
const OTHER_TOKEN: &str = "YOUR_FAKE_UNSELECTED_MCP_TOKEN_19";
const NUMERIC_TOKEN: &str = "7319420918273645";

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Env {
    root: PathBuf,
    home: PathBuf,
    data: PathBuf,
    server: PathBuf,
}
impl Env {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "am-mcp-native-{tag}-{}",
            agent_manager_engine::store::uuid_v4()
        ));
        #[cfg(windows)]
        common::private_fixture_root(&root);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&root)
                .unwrap();
        }
        let home = root.join("home");
        let data = root.join("data");
        let server = root.join("native tools with spaces/MCP 服务.exe");
        std::fs::create_dir_all(server.parent().unwrap()).unwrap();
        std::fs::create_dir(&home).unwrap();
        std::fs::copy(std::env::current_exe().unwrap(), &server).unwrap();
        Self {
            root,
            home,
            data,
            server,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"));
        command.env_clear();
        #[cfg(windows)]
        let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
        #[cfg(unix)]
        let system = PathBuf::from("/usr/bin");
        let path = std::env::join_paths([self.server.parent().unwrap(), system.as_path()]).unwrap();
        command
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("AGENT_MANAGER_DATA_DIR", &self.data)
            .env("PATH", path)
            .env("HTTP_PROXY", "")
            .env("HTTPS_PROXY", "")
            .env("ALL_PROXY", "")
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("OPENAI_API_KEY", OTHER_TOKEN)
            .env("AM_UNSELECTED_MCP_TOKEN", OTHER_TOKEN);
        #[cfg(windows)]
        for name in ["SystemRoot", "WINDIR", "COMSPEC", "PATHEXT"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
    }

    fn run(&self, args: &[&str]) -> Value {
        let stdout = self.root.join("cli-stdout");
        let stderr = self.root.join("cli-stderr");
        let mut command = self.command();
        command
            .args(args)
            .arg("--json")
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&stdout).unwrap())
            .stderr(std::fs::File::create(&stderr).unwrap());
        let mut child = OwnedChild(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(60);
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "Owned CLI exceeded its deadline");
            std::thread::sleep(Duration::from_millis(10));
        };
        let output = std::fs::read(&stdout).unwrap();
        let error = std::fs::read(&stderr).unwrap();
        assert!(output.len() < 1024 * 1024 && error.len() < 1024 * 1024);
        assert!(
            !String::from_utf8_lossy(&output).contains(TOKEN),
            "CLI result leaked a resolved test secret"
        );
        assert!(
            !String::from_utf8_lossy(&error).contains(TOKEN),
            "CLI diagnostic leaked a resolved test secret"
        );
        assert!(!String::from_utf8_lossy(&output).contains(NUMERIC_TOKEN));
        assert!(!String::from_utf8_lossy(&error).contains(NUMERIC_TOKEN));
        assert!(
            status.success(),
            "CLI failed: {} / {}",
            String::from_utf8_lossy(&output),
            String::from_utf8_lossy(&error)
        );
        serde_json::from_slice(&output).unwrap()
    }

    fn config(&self, mode: &str, batch: bool) -> (PathBuf, Vec<String>) {
        let receipt = self.root.join("receipt");
        std::fs::create_dir(&receipt).unwrap();
        let mut args = vec![
            "--mcp-fixture".into(),
            mode.into(),
            receipt.to_str().unwrap().into(),
            "literal spaces".into(),
            "汉字".into(),
        ];
        if !batch {
            args.extend([
                "quote\"value".into(),
                "percent%value".into(),
                "amp&value".into(),
            ]);
        }
        let entry = if batch {
            let mut wrapped = vec!["/c".into(), "npx".into()];
            wrapped.extend(args.clone());
            json!({"command":"cmd","args":wrapped})
        } else {
            json!({"command":self.server,"args":args})
        };
        let mut entry = entry;
        entry["env"] = json!({"AM_NATIVE_MCP_TOKEN":"${AM_NATIVE_MCP_TOKEN}","AM_NUMERIC_MCP_TOKEN":"${AM_NUMERIC_MCP_TOKEN}"});
        entry["cwd"] = json!(receipt);
        entry["startup_timeout_ms"] = json!(10_000);
        entry["tool_timeout_sec"] = json!(1);
        std::fs::write(
            self.home.join(".claude.json"),
            json!({"mcpServers":{"native":entry}}).to_string(),
        )
        .unwrap();
        let source = self.home.join("secrets.env");
        std::fs::write(
            &source,
            format!("AM_NATIVE_MCP_TOKEN={TOKEN}\nAM_UNSELECTED_MCP_TOKEN={OTHER_TOKEN}\nAM_NUMERIC_MCP_TOKEN={NUMERIC_TOKEN}\n"),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let registered = self.run(&["secrets:source", "--path", source.to_str().unwrap()]);
        assert_eq!(registered["ok"], true, "{registered}");
        (receipt, args)
    }
}
impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn database_bytes(data: &Path) -> Vec<u8> {
    let mut paths: Vec<_> = std::fs::read_dir(data)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("native-manager.db")
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .flat_map(|path| std::fs::read(path).unwrap())
        .collect()
}

fn stdio_case(mode: &str, batch: bool) {
    let env = Env::new(mode);
    #[cfg(windows)]
    if batch {
        std::fs::copy(
            &env.server,
            env.server.parent().unwrap().join("mcp-fixture.exe"),
        )
        .unwrap();
        std::fs::write(
            env.server.parent().unwrap().join("npx.cmd"),
            "@echo off\r\n\"%~dp0mcp-fixture.exe\" %*\r\n",
        )
        .unwrap();
    }
    let (receipt, args) = env.config(mode, batch);
    let config = std::fs::read(env.home.join(".claude.json")).unwrap();
    let secrets = std::fs::read(env.home.join("secrets.env")).unwrap();
    let db = database_bytes(&env.data);
    let result = env.run(&[
        "mcp:check",
        "--only",
        "mcp:native",
        "--agent",
        "claude-code",
    ]);
    assert_eq!(result["ok"], true, "{result}");
    let data = &result["data"];
    assert_eq!(data["startup_timeout_sec"].as_f64(), Some(10.0));
    assert_eq!(data["tool_timeout_sec"].as_f64(), Some(1.0));
    let success = matches!(mode, "ok" | "echo" | "no-tools");
    assert_eq!(data["ok"], success, "{result}");
    assert_eq!(
        data["stage"],
        if success {
            "done"
        } else if matches!(mode, "invalid" | "exit") {
            "initialize"
        } else {
            "tools"
        },
        "{result}"
    );
    let requests: Vec<Value> = std::fs::read_to_string(receipt.join("requests.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        requests.len(),
        if matches!(mode, "invalid" | "exit") {
            1
        } else if mode == "no-tools" {
            2
        } else {
            3
        }
    );
    assert_eq!(requests[0]["method"], "initialize");
    assert_eq!(requests[0]["id"], 1);
    assert_eq!(requests[0]["params"]["protocolVersion"], "2025-06-18");
    if requests.len() > 1 {
        assert_eq!(requests[1]["method"], "notifications/initialized");
        assert!(requests[1].get("id").is_none());
    }
    if requests.len() > 2 {
        assert_eq!(requests[2]["method"], "tools/list");
        assert_eq!(requests[2]["id"], 2);
    }
    if matches!(mode, "ok" | "echo") {
        assert_eq!(data["tool_count"], 2);
    }
    if mode == "ok" {
        assert_eq!(data["tools"], json!(["native_tool", "other_tool"]));
    }
    if mode == "no-tools" {
        assert_eq!(data["tool_count"], Value::Null);
    }
    if mode == "exit" {
        assert_eq!(data["exit_code"], 7);
    }
    if mode == "timeout" {
        assert_eq!(data["exit_code"], Value::Null);
    }
    if mode == "echo" {
        assert_eq!(data["server_info"]["nested"][0]["echo"], "***");
        assert_eq!(data["server_info"]["***"][1], true);
        assert_eq!(data["tools"][0], "native_***");
        assert_eq!(data["server_info"]["numeric"], "***");
        assert_eq!(data["server_info"]["many"][9_999]["value"], "***");
        assert_eq!(data["server_info"]["many"][9_999]["index"], 9_999);
    }
    let observed: Value =
        serde_json::from_slice(&std::fs::read(receipt.join("context.json")).unwrap()).unwrap();
    assert_eq!(observed["argv"], json!(args));
    assert_eq!(
        std::fs::canonicalize(observed["cwd"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(&receipt).unwrap()
    );
    assert_eq!(observed["selected"], TOKEN);
    assert_eq!(observed["unselected"], Value::Null);
    assert_eq!(observed["provider"], Value::Null);
    assert_eq!(observed["home"], env.home.to_str().unwrap());
    assert_eq!(
        std::fs::read(env.home.join(".claude.json")).unwrap(),
        config
    );
    assert_eq!(
        std::fs::read(env.home.join("secrets.env")).unwrap(),
        secrets
    );
    assert_eq!(database_bytes(&env.data), db);
    assert!(!String::from_utf8_lossy(&db).contains(TOKEN));
    println!("native MCP stdio {mode} (batch={batch}): passed");
}

fn fixture(args: &[String]) {
    let mode = &args[1];
    let root = PathBuf::from(&args[2]);
    let token = std::env::var("AM_NATIVE_MCP_TOKEN").unwrap();
    std::fs::write(
        root.join("context.json"),
        json!({
            "argv": args, "cwd": std::env::current_dir().unwrap(),
            "selected": token, "unselected": std::env::var("AM_UNSELECTED_MCP_TOKEN").ok(),
            "provider": std::env::var("OPENAI_API_KEY").ok(), "home": std::env::var("HOME").ok()
        })
        .to_string(),
    )
    .unwrap();
    let mut log = std::fs::File::create(root.join("requests.jsonl")).unwrap();
    let mut input = std::io::stdin().lock();
    let mut receive = || {
        let mut line = String::new();
        assert!(input.read_line(&mut line).unwrap() > 0);
        log.write_all(line.as_bytes()).unwrap();
        log.flush().unwrap();
        serde_json::from_str::<Value>(&line).unwrap()
    };
    let init = receive();
    assert_eq!(init["method"], "initialize");
    if mode == "exit" {
        eprintln!("fixture failure: {token}");
        std::process::exit(7);
    }
    let capabilities = if mode == "no-tools" {
        json!({})
    } else {
        json!({"tools":{}})
    };
    let server = if mode == "echo" {
        let numeric: u64 = std::env::var("AM_NUMERIC_MCP_TOKEN")
            .unwrap()
            .parse()
            .unwrap();
        let many: Vec<_> = (0..10_000)
            .map(|index| json!({"value":token,"index":index}))
            .collect();
        json!({"name":format!("native_{token}"),"version":"1","nested":[{"echo":token}],token.clone():[token,true,7,null],"numeric":numeric,"many":many})
    } else {
        json!({"name":"native-fixture","version":"1"})
    };
    let result = if mode == "invalid" {
        json!({"capabilities":[],"serverInfo":server})
    } else {
        json!({"protocolVersion":"2025-06-18","capabilities":capabilities,"serverInfo":server})
    };
    println!(
        "{}",
        json!({"jsonrpc":"2.0","id":init["id"],"result":result})
    );
    std::io::stdout().flush().unwrap();
    if mode == "invalid" {
        return;
    }
    let initialized = receive();
    assert_eq!(initialized["method"], "notifications/initialized");
    if mode == "no-tools" {
        return;
    }
    let tools = receive();
    assert_eq!(tools["method"], "tools/list");
    if mode == "timeout" {
        std::thread::sleep(Duration::from_secs(10));
        return;
    }
    let response = if mode == "tools-error" {
        json!({"error":{"code":-32603,"message":format!("fixture rejected: {token}")}})
    } else if mode == "missing-tools" {
        json!({"result":{}})
    } else {
        json!({"result":{"tools":[{"name":if mode == "echo" { format!("native_{token}") } else { "native_tool".into() }},{"name":"other_tool"}]}})
    };
    let mut response = response;
    response["jsonrpc"] = json!("2.0");
    response["id"] = tools["id"].clone();
    println!("{response}");
    std::io::stdout().flush().unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--mcp-fixture") {
        fixture(&args);
        return;
    }
    for mode in [
        "ok",
        "no-tools",
        "invalid",
        "tools-error",
        "missing-tools",
        "exit",
        "timeout",
        "echo",
    ] {
        stdio_case(mode, false);
    }
    #[cfg(windows)]
    stdio_case("ok", true);
}
