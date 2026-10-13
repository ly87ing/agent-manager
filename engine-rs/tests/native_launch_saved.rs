use agent_manager_engine::{store, Context};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, process::Command};

struct Fixture {
    root: PathBuf,
    ctx: Context,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("launch-saved-{}", store::uuid_v4()));
        fs::create_dir_all(root.join("home")).unwrap();
        let root = root.canonicalize().unwrap();
        let ctx = Context {
            home_dir: root.join("home"),
            data_dir: root.join("data"),
        };
        for file in [
            ".zshrc",
            ".zprofile",
            ".bashrc",
            "Microsoft.PowerShell_profile.ps1",
        ] {
            fs::write(ctx.home_dir.join(file), format!("# preserve {file}\n")).unwrap();
        }
        Self { root, ctx }
    }
    fn run(&self, command: &str, args: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
            .args([command, "--json"])
            .args(args)
            .env("HOME", &self.ctx.home_dir)
            .env("USERPROFILE", &self.ctx.home_dir)
            .env("AGENT_MANAGER_DATA_DIR", &self.ctx.data_dir)
            .output()
            .unwrap();
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("CLI did not return JSON: {:?}", output.status))
    }
    fn unchanged_home(&self) {
        for file in [
            ".zshrc",
            ".zprofile",
            ".bashrc",
            "Microsoft.PowerShell_profile.ps1",
        ] {
            assert_eq!(
                fs::read_to_string(self.ctx.home_dir.join(file)).unwrap(),
                format!("# preserve {file}\n")
            );
        }
        assert_eq!(fs::read_dir(&self.ctx.home_dir).unwrap().count(), 4);
        assert!(!self.root.join("executed").exists());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn spec(platform: &str, shell: &str) -> Value {
    json!({"platform":platform,"shell":shell,"kind":"command","target":"not-installed",
        "args":["O'Brien", "$(touch executed)", ""],
        "variants":[{"name":"custom-sg","timezone":"Asia/Singapore"},
            {"name":"custom-la","timezone":"America/Los_Angeles"}]})
}

#[test]
fn saved_recipes_survive_cli_restarts_without_applying_or_executing() {
    let f = Fixture::new();
    assert_eq!(f.run("launch:list", &[])["data"]["items"], json!([]));
    assert!(!f.ctx.data_dir.exists());
    for (platform, shell) in [
        ("macos", "zsh"),
        ("linux", "bash"),
        ("windows", "powershell"),
    ] {
        let recipe = spec(platform, shell);
        let saved = f.run("launch:save", &["--spec", &recipe.to_string()]);
        assert_eq!(saved["ok"], true, "{saved}");
        let item = &saved["data"]["item"];
        let id = item["id"].as_str().unwrap();
        let base = item["revision"].as_str().unwrap();
        assert_eq!(item["recipe"], recipe);
        let listed = f.run("launch:list", &[]);
        assert!(listed["data"]["items"].as_array().unwrap().contains(item));
        f.unchanged_home();
        let mut next = recipe.clone();
        next["variants"][0]["name"] = json!("updated-sg");
        let updated = f.run(
            "launch:save",
            &["--id", id, "--base", base, "--spec", &next.to_string()],
        );
        assert_eq!(updated["ok"], true, "{updated}");
        let new_base = updated["data"]["item"]["revision"].as_str().unwrap();
        assert_eq!(
            f.run(
                "launch:save",
                &["--id", id, "--base", base, "--spec", &recipe.to_string()]
            )["ok"],
            false
        );
        assert_eq!(
            f.run("launch:delete", &["--id", id, "--base", base])["ok"],
            false
        );
        assert_eq!(
            f.run("launch:list", &[])["data"]["items"][0]["recipe"],
            next
        );
        assert_eq!(
            f.run("launch:delete", &["--id", id, "--base", new_base])["ok"],
            true
        );
        assert_eq!(f.run("launch:list", &[])["data"]["items"], json!([]));
        f.unchanged_home();
    }
    let conn = store::open_readonly(&f.ctx).unwrap().unwrap();
    let details: String = conn
        .query_row(
            "SELECT group_concat(coalesce(detail_json, ''), '') FROM operation_logs",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!details.contains("O'Brien") && !details.contains("not-installed"));
}

#[test]
fn rejected_saves_and_missing_deletes_do_not_create_state() {
    let f = Fixture::new();
    let mut secret = spec("macos", "zsh");
    secret["args"] = json!([format!("sk-{}", "x".repeat(30))]);
    for args in [
        vec!["--spec".into(), secret.to_string()],
        vec![
            "--spec".into(),
            spec("linux", "zsh").to_string(),
            "--confirm".into(),
        ],
        vec![
            "--spec".into(),
            spec("macos", "zsh").to_string(),
            "--base".into(),
            "missing".into(),
        ],
    ] {
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();
        assert_eq!(f.run("launch:save", &args)["ok"], false);
    }
    assert_eq!(
        f.run(
            "launch:delete",
            &["--id", "absent", "--base", &"0".repeat(64)]
        )["ok"],
        false
    );
    assert!(!f.ctx.data_dir.exists());
    f.unchanged_home();
}

#[test]
fn existing_database_reads_are_read_only_and_failed_audits_roll_back_save() {
    let f = Fixture::new();
    let conn = store::open_write(&f.ctx).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER reject_launch_audit BEFORE INSERT ON operation_logs
        WHEN NEW.operation_type = 'launch:save' BEGIN SELECT RAISE(ABORT, 'audit failure'); END;",
    )
    .unwrap();
    assert_eq!(f.run("launch:list", &[])["data"]["items"], json!([]));
    let exists = || {
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='launch_recipes'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    assert_eq!(exists(), 0);
    let result = f.run(
        "launch:save",
        &["--spec", &spec("linux", "bash").to_string()],
    );
    assert_eq!(result["ok"], false);
    assert_eq!(exists(), 0);
    assert_eq!(f.run("launch:list", &[])["data"]["items"], json!([]));
    f.unchanged_home();
}
