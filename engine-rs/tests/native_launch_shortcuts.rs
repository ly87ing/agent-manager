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
            "launch-shortcuts-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let ctx = Context {
            home_dir: root.join("home"),
            data_dir: root.join("data"),
        };
        std::fs::create_dir_all(&ctx.home_dir).unwrap();
        std::fs::write(
            ctx.home_dir.join(".zprofile"),
            "# unrelated login settings\n",
        )
        .unwrap();
        Self { root, ctx }
    }
    fn run(&self, command: &str, mut args: Vec<String>) -> agent_manager_engine::Envelope {
        // This fixture exercises zsh on every Unix host, including Linux where
        // the product's default shell is Bash.
        if !cfg!(windows) && !args.iter().any(|arg| arg == "--shell") {
            args.extend(["--shell".into(), "zsh".into()]);
        }
        execute(
            &self.ctx,
            &Request {
                command: format!("launch:{command}"),
                args,
                run_id: "shortcut-test".into(),
            },
        )
    }
    fn spec(&self) -> Value {
        json!({"platform":std::env::consts::OS,"shell":if cfg!(windows){"powershell"}else{"zsh"},
            "kind":"app","target":"Example application","args":[],
            "variants":[{"name":"example-sg","timezone":"Asia/Singapore"},
                {"name":"example-la","timezone":"America/Los_Angeles"}]})
    }
    fn base(&self) -> String {
        let status = self.run("status", vec![]);
        assert!(status.ok, "{:?}", status.errors);
        status.data["base"].as_str().unwrap().into()
    }
    fn args(&self, base: &str) -> Vec<String> {
        vec![
            "--spec".into(),
            self.spec().to_string(),
            "--base".into(),
            base.into(),
        ]
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[cfg(windows)]
#[test]
fn windows_unmanaged_scripts_are_preserved_and_failed_removal_is_restored() {
    let f = Fixture::new();
    let directory = f.ctx.home_dir.join(".agent-manager/launchers");
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("example-sg.ps1");
    std::fs::write(&file, "# user-owned script\n").unwrap();
    assert!(!f.run("preview", f.args(&f.base())).ok);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "# user-owned script\n"
    );
    std::fs::remove_file(&file).unwrap();
    let mut args = f.args(&f.base());
    args.push("--confirm".into());
    assert!(f.run("apply", args).ok);
    let original = std::fs::read(&file).unwrap();
    let conn = agent_manager_engine::store::open_write(&f.ctx).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_launch_audit BEFORE INSERT ON operation_logs BEGIN SELECT RAISE(ABORT,'audit unavailable'); END;").unwrap();
    let result = f.run(
        "apply",
        vec![
            "--remove".into(),
            "example-sg".into(),
            "--base".into(),
            f.base(),
            "--confirm".into(),
        ],
    );
    assert!(!result.ok);
    assert_eq!(std::fs::read(file).unwrap(), original);
}

#[test]
fn preview_and_unconfirmed_apply_are_write_free_then_apply_reads_back() {
    let f = Fixture::new();
    let base = f.base();
    let args = f.args(&base);
    let preview = f.run("preview", args.clone());
    assert!(preview.ok, "{:?}", preview.errors);
    assert!(!f.ctx.data_dir.exists());
    assert!(!f.run("apply", args.clone()).ok);
    assert!(!f.ctx.data_dir.exists());
    let mut args = args;
    args.push("--confirm".into());
    let applied = f.run("apply", args);
    assert!(applied.ok, "{:?}", applied.errors);
    assert_eq!(
        applied.data["status"]["entries"].as_array().unwrap().len(),
        2
    );
    for file in preview.data["files"].as_array().unwrap() {
        assert_eq!(
            std::fs::read_to_string(file["path"].as_str().unwrap()).unwrap(),
            file["content"].as_str().unwrap()
        );
    }
    assert_eq!(
        std::fs::read_to_string(f.ctx.home_dir.join(".zprofile")).unwrap(),
        "# unrelated login settings\n"
    );
    let new_base = f.base();
    assert_ne!(base, new_base);
    let removed = f.run(
        "apply",
        vec![
            "--remove".into(),
            "example-sg".into(),
            "--base".into(),
            new_base,
            "--confirm".into(),
        ],
    );
    assert!(removed.ok, "{:?}", removed.errors);
    assert_eq!(
        removed.data["status"]["entries"].as_array().unwrap().len(),
        1
    );
}

#[cfg(unix)]
#[test]
fn stale_preview_and_outside_alias_conflicts_preserve_all_bytes() {
    let f = Fixture::new();
    let rc = f.ctx.home_dir.join(".zshrc");
    let base = f.base();
    std::fs::write(&rc, "# changed outside the tool\n").unwrap();
    let mut args = f.args(&base);
    args.push("--confirm".into());
    assert!(!f.run("apply", args).ok);
    assert!(!f.ctx.data_dir.exists());
    std::fs::write(&rc, "alias example-sg='user-owned'\n").unwrap();
    let result = f.run("preview", f.args(&f.base()));
    assert!(!result.ok);
    assert_eq!(
        std::fs::read_to_string(rc).unwrap(),
        "alias example-sg='user-owned'\n"
    );
}

#[cfg(unix)]
#[test]
fn audit_failure_rolls_back_the_actual_shell_file() {
    let f = Fixture::new();
    let rc = f.ctx.home_dir.join(".zshrc");
    let original = "# keep exactly\nexport EXAMPLE=1\n";
    std::fs::write(&rc, original).unwrap();
    let conn = agent_manager_engine::store::open_write(&f.ctx).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_launch_audit BEFORE INSERT ON operation_logs BEGIN SELECT RAISE(ABORT,'audit unavailable'); END;").unwrap();
    let mut args = f.args(&f.base());
    args.push("--confirm".into());
    let result = f.run("apply", args);
    assert!(!result.ok);
    assert_eq!(std::fs::read_to_string(rc).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn protected_identity_links_are_rejected_and_ordinary_dotfile_links_survive() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let rc = f.ctx.home_dir.join(".zshrc");
    let identity = f.ctx.home_dir.join(".codex/config.toml");
    std::fs::create_dir_all(identity.parent().unwrap()).unwrap();
    std::fs::write(&identity, "# identity must not change\n").unwrap();
    symlink(&identity, &rc).unwrap();
    assert!(!f.run("status", vec![]).ok);
    assert!(!f.ctx.data_dir.exists());
    std::fs::remove_file(&rc).unwrap();
    let dotfile = f.ctx.home_dir.join("dotfiles/zshrc");
    std::fs::create_dir_all(dotfile.parent().unwrap()).unwrap();
    std::fs::write(&dotfile, "# ordinary dotfile\n").unwrap();
    symlink(&dotfile, &rc).unwrap();
    let mut args = f.args(&f.base());
    args.push("--confirm".into());
    let result = f.run("apply", args);
    assert!(result.ok, "{:?}", result.errors);
    assert!(rc.is_symlink());
    assert!(std::fs::read_to_string(dotfile)
        .unwrap()
        .starts_with("# ordinary dotfile\n"));
    assert_eq!(
        std::fs::read_to_string(identity).unwrap(),
        "# identity must not change\n"
    );
}

#[cfg(unix)]
#[test]
fn real_cli_applies_only_after_confirmation_and_never_writes_zprofile() {
    let f = Fixture::new();
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"));
    cmd.env("HOME", &f.ctx.home_dir)
        .env("AGENT_MANAGER_DATA_DIR", &f.ctx.data_dir)
        .env_remove("ZDOTDIR");
    let mut args = f.args("missing");
    args.push("--confirm".into());
    let output = cmd
        .arg("launch:apply")
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{result}");
    assert!(f.ctx.home_dir.join(".zshrc").exists());
    assert_eq!(
        std::fs::read_to_string(f.ctx.home_dir.join(".zprofile")).unwrap(),
        "# unrelated login settings\n"
    );
}
