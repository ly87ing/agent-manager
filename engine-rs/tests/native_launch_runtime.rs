use agent_manager_engine::{execute, Context, Request};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    ctx: Context,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "launch-runtime-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        Self {
            ctx: Context {
                home_dir: root.join("home"),
                data_dir: root.join("data"),
            },
            root,
        }
    }
    fn call(&self, operation: &str, args: Vec<String>) -> agent_manager_engine::Envelope {
        execute(
            &self.ctx,
            &Request {
                command: format!("launch:{operation}"),
                args,
                run_id: "runtime-test".into(),
            },
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn spec(target: &str) -> Value {
    json!({"platform":std::env::consts::OS,"shell":if cfg!(windows) {"powershell"} else {"zsh"},
        "kind":"app","target":target,"args":[],"variants":[{"name":"example-sg","timezone":"Asia/Singapore"}]})
}

#[test]
fn catalog_is_read_only_and_rejects_extra_arguments() {
    let f = Fixture::new();
    let result = f.call("catalog", vec![]);
    assert!(result.ok, "{:?}", result.errors);
    assert_eq!(result.data["platform"], std::env::consts::OS);
    assert!(!f.call("catalog", vec!["--confirm".into()]).ok);
    assert!(!f.ctx.home_dir.exists());
    assert!(!f.ctx.data_dir.exists());
}

#[test]
fn launch_requires_confirmation_and_rejects_cli_and_duplicate_flags_before_state() {
    let f = Fixture::new();
    let mut recipe = spec("/does/not/exist");
    let args = vec![
        "--spec".into(),
        recipe.to_string(),
        "--name".into(),
        "example-sg".into(),
    ];
    let result = f.call("run", args.clone());
    assert!(!result.ok);
    assert!(result.errors[0].message.contains("--confirm"));
    recipe["kind"] = json!("command");
    let result = f.call(
        "run",
        vec![
            "--spec".into(),
            recipe.to_string(),
            "--name".into(),
            "example-sg".into(),
            "--confirm".into(),
        ],
    );
    assert!(!result.ok);
    assert!(result.errors[0].message.contains("terminal"));
    let mut duplicate = args;
    duplicate.extend(["--name".into(), "other".into(), "--confirm".into()]);
    assert!(!f.call("run", duplicate).ok);
    assert!(!f.ctx.data_dir.exists());
}

#[test]
fn real_cli_launches_an_independent_application_with_literal_args_and_scoped_timezone() {
    let f = Fixture::new();
    std::fs::create_dir_all(&f.root).unwrap();
    let source = f.root.join("probe.rs");
    std::fs::write(&source, r#"fn main() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        std::fs::write(&args[0], format!("{}\n{:?}", std::env::var("TZ").unwrap_or_default(), &args[1..])).unwrap();
        let release = std::path::Path::new(&args[1]).with_extension("release");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !release.exists() && std::time::Instant::now() < deadline { std::thread::sleep(std::time::Duration::from_millis(20)); }
        std::fs::write(&args[1], "survived").unwrap();
    }"#).unwrap();
    let executable = if cfg!(target_os = "macos") {
        f.root.join("Probe.app/Contents/MacOS/probe")
    } else {
        f.root
            .join(if cfg!(windows) { "probe.exe" } else { "probe" })
    };
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    assert!(std::process::Command::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap()
        .success());
    if cfg!(target_os = "macos") {
        std::fs::write(f.root.join("Probe.app/Contents/Info.plist"), r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>probe</string><key>CFBundleIdentifier</key><string>test.agentmanager.launchprobe</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>"#).unwrap();
    }
    let ready = f.root.join("ready");
    let done = f.root.join("done");
    let target = if cfg!(target_os = "macos") {
        f.root.join("Probe.app")
    } else {
        executable.clone()
    };
    let mut recipe = spec(target.to_str().unwrap());
    recipe["args"] = json!([ready, done, "O'Brien", "$(touch unexpected)", ""]);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
        .env("HOME", &f.ctx.home_dir)
        .env("AGENT_MANAGER_DATA_DIR", &f.ctx.data_dir)
        .args([
            "launch:run",
            "--spec",
            &recipe.to_string(),
            "--name",
            "example-sg",
            "--confirm",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["data"]["timezone_verified"], false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !ready.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    let content = std::fs::read_to_string(&ready).unwrap();
    assert!(content.starts_with("Asia/Singapore\n"), "{content}");
    assert!(
        content.contains("O'Brien")
            && content.contains("$(touch unexpected)")
            && content.ends_with("\"\"]"),
        "{content}"
    );
    let running = f.call(
        "run",
        vec![
            "--spec".into(),
            recipe.to_string(),
            "--name".into(),
            "example-sg".into(),
            "--confirm".into(),
        ],
    );
    assert!(!running.ok, "already-running applications must not restart");
    assert!(
        running.errors[0].message.contains("already running"),
        "{:?}",
        running.errors
    );
    std::fs::write(done.with_extension("release"), "release").unwrap();
    while !done.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    assert_eq!(std::fs::read_to_string(done).unwrap(), "survived");
    assert!(!f.root.join("unexpected").exists());
}
