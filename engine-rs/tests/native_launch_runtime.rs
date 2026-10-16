use agent_manager_engine::{execute, Context, Request};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[cfg(target_os = "linux")]
#[test]
fn linux_catalog_reads_xdg_overrides_and_fixed_arguments_without_writes() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let user = f.root.join("user-data/applications");
    let system = f.root.join("system-data/applications");
    std::fs::create_dir_all(&user).unwrap();
    std::fs::create_dir_all(&system).unwrap();
    let executable = f.root.join("sample app");
    std::fs::write(&executable, b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let entry = |name: &str, args: &str| {
        format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec=\"{}\" {args}\n",
            executable.display()
        )
    };
    std::fs::write(
        system.join("hidden.desktop"),
        entry("Hidden system app", "%U"),
    )
    .unwrap();
    std::fs::write(
        user.join("hidden.desktop"),
        "[Desktop Entry]\nHidden=true\n",
    )
    .unwrap();
    std::fs::write(
        user.join("sample.desktop"),
        entry("Sample", "--label \"two words\" %F"),
    )
    .unwrap();
    std::fs::write(
        user.join("private.desktop"),
        entry("Sample private", "--private"),
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
        .args(["launch:catalog", "--json"])
        .env("HOME", &f.ctx.home_dir)
        .env("AGENT_MANAGER_DATA_DIR", &f.ctx.data_dir)
        .env("XDG_DATA_HOME", user.parent().unwrap())
        .env("XDG_DATA_DIRS", system.parent().unwrap())
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], true, "{value}");
    let apps = value["data"]["apps"].as_array().unwrap();
    assert_eq!(apps.len(), 2, "{value}");
    assert_eq!(apps[0]["args"], json!(["--label", "two words"]));
    assert_eq!(apps[1]["args"], json!(["--private"]));
    assert!(!f.ctx.home_dir.exists());
    assert!(!f.ctx.data_dir.exists());
}
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
        "kind":"app","target":target,"args":[],
        "env":[{"name":"LANG","value":"en_US.UTF-8"},{"name":"CUSTOM_ENV","value":"literal $HOME"}],
        "variants":[{"name":"example-sg","timezone":"Asia/Singapore"}]})
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
fn rejected_system_launch_records_its_failure_without_command_details() {
    let f = Fixture::new();
    std::fs::create_dir_all(&f.root).unwrap();
    let target = if cfg!(target_os = "macos") {
        let app = f.root.join("Broken.app");
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        std::fs::write(app.join("Contents/Info.plist"), r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>missing</string><key>CFBundleIdentifier</key><string>test.agentmanager.brokenprobe</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>"#).unwrap();
        app
    } else {
        let path = f.root.join(if cfg!(windows) {
            "broken.exe"
        } else {
            "broken"
        });
        std::fs::write(&path, "not an executable").unwrap();
        path
    };
    let mut recipe = spec(target.to_str().unwrap());
    recipe["args"] = json!(["private-argument"]);
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
    assert!(!result.ok, "{result:?}");
    let detail = execute(
        &f.ctx,
        &Request {
            command: "runs:get".into(),
            args: vec!["--run".into(), "runtime-test".into()],
            run_id: "readback".into(),
        },
    );
    assert!(detail.ok, "{detail:?}");
    let operations = detail.data["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 2, "{detail:?}");
    assert_eq!(operations[0]["detail"]["state"], "requested");
    assert_eq!(operations[1]["detail"]["state"], "failed");
    assert!(!detail.data.to_string().contains("private-argument"));
    assert!(!detail.data.to_string().contains(target.to_str().unwrap()));
}

#[test]
fn real_cli_launches_an_independent_application_with_literal_args_and_scoped_timezone() {
    check_real_cli_launch(false);
}

#[test]
fn accepted_launch_preserves_its_outcome_when_completion_audit_fails() {
    check_real_cli_launch(true);
}

fn check_real_cli_launch(reject_completion_audit: bool) {
    let f = Fixture::new();
    std::fs::create_dir_all(&f.root).unwrap();
    let source = f.root.join("probe.rs");
    std::fs::write(&source, r#"fn publish(path: &str, content: &str) {
        let target = std::path::Path::new(path);
        let staging = target.with_extension("writing");
        std::fs::write(&staging, content).unwrap();
        std::fs::rename(staging, target).unwrap();
    }
    fn main() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        publish(&args[0], &format!("{}\n{}\n{}\n{:?}", std::env::var("TZ").unwrap_or_default(), std::env::var("LANG").unwrap_or_default(), std::env::var("CUSTOM_ENV").unwrap_or_default(), &args[1..]));
        let release = std::path::Path::new(&args[1]).with_extension("release");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !release.exists() && std::time::Instant::now() < deadline { std::thread::sleep(std::time::Duration::from_millis(20)); }
        publish(&args[1], "survived");
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
    if reject_completion_audit {
        let db = agent_manager_engine::store::open_write(&f.ctx).unwrap();
        db.execute_batch("CREATE TRIGGER reject_launch_result BEFORE INSERT ON operation_logs
            WHEN NEW.operation_type='launch:run' AND json_extract(NEW.detail_json, '$.state')='accepted'
            BEGIN SELECT RAISE(ABORT, 'fixture completion failure'); END;").unwrap();
    }
    let output_file = f.root.join("launch-result.json");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
        .env("HOME", &f.ctx.home_dir)
        .env("AGENT_MANAGER_DATA_DIR", &f.ctx.data_dir)
        .stdout(std::fs::File::create(&output_file).unwrap())
        .args([
            "launch:run",
            "--spec",
            &recipe.to_string(),
            "--name",
            "example-sg",
            "--confirm",
            "--json",
        ])
        .status()
        .unwrap();
    let result: Value = serde_json::from_slice(&std::fs::read(output_file).unwrap()).unwrap();
    assert!(status.success(), "CLI launch request failed: {status}; {result}");
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["data"]["timezone_verified"], false);
    assert_eq!(result["data"]["audit_recorded"], !reject_completion_audit);
    assert_eq!(
        result["warnings"].as_array().unwrap().len(),
        usize::from(reject_completion_audit)
    );
    let history = execute(
        &f.ctx,
        &Request {
            command: "runs:get".into(),
            args: vec!["--run".into(), result["run_id"].as_str().unwrap().into()],
            run_id: "history".into(),
        },
    );
    assert!(history.ok, "{history:?}");
    let operations = history.data["operations"].as_array().unwrap();
    assert_eq!(
        operations.len(),
        if reject_completion_audit { 1 } else { 2 }
    );
    assert_eq!(
        operations.last().unwrap()["detail"]["state"],
        if reject_completion_audit {
            "requested"
        } else {
            "accepted"
        }
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !ready.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    let content = std::fs::read_to_string(&ready).unwrap();
    assert!(
        content.starts_with("Asia/Singapore\nen_US.UTF-8\nliteral $HOME\n"),
        "{content:?}"
    );
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
