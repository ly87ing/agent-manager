//! Portable uv-result acceptance; this binary also serves as the isolated installer.
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

fn entry_name() -> &'static str {
    if cfg!(windows) {
        "tool-a.exe"
    } else {
        "tool-a"
    }
}

fn site(package: &Path) -> PathBuf {
    package.join(if cfg!(windows) {
        "Lib/site-packages"
    } else {
        "lib/python3.13/site-packages"
    })
}

fn install(package: &Path, bin: &Path, version: &str, case: &str) {
    let distribution = site(package).join(format!("tool_a-{version}.dist-info"));
    let scripts = package.join(if cfg!(windows) { "Scripts" } else { "bin" });
    std::fs::create_dir_all(&distribution).unwrap();
    std::fs::create_dir_all(&scripts).unwrap();
    let metadata = format!("Metadata-Version: 2.4\nName: TOOL_a\nVersion: {version}\n\n");
    if case != "missing" {
        std::fs::write(distribution.join("METADATA"), &metadata).unwrap();
    }
    if case == "duplicate" {
        let other = site(package).join("TOOL_A-0.0.1.dist-info");
        std::fs::create_dir(&other).unwrap();
    }
    if case == "malformed" {
        std::fs::write(
            distribution.join("METADATA"),
            format!(
                "Metadata-Version: 2.4\nName: tool-a\nVersion: {version}\nVERSION: {version}\n\n"
            ),
        )
        .unwrap();
    }
    std::fs::write(package.join("declared-version"), version).unwrap();
    if case != "missing-bin" {
        let source = scripts.join(entry_name());
        std::fs::copy(std::env::current_exe().unwrap(), &source).unwrap();
        #[cfg(windows)]
        std::fs::copy(&source, bin.join(entry_name())).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&source, bin.join(entry_name())).unwrap();
    }
    std::fs::write(
        package.join("uv-receipt.toml"),
        format!(
            "[tool]\nentrypoints = [{{ name = 'tool-a', install-path = {} }}]\n",
            serde_json::to_string(&bin.join(entry_name()).to_string_lossy()).unwrap()
        ),
    )
    .unwrap();
}

fn uv_fixture(args: &[String]) {
    std::fs::write(
        std::env::var_os("AM_TEST_UV_EXECUTED").unwrap(),
        args.join(" "),
    )
    .unwrap();
    assert_eq!(args.len(), 5);
    assert_eq!(
        &args[..4],
        ["--no-python-downloads", "tool", "install", "--force"]
    );
    assert!(args[4].starts_with("tool-a=="));
    let control: Value = serde_json::from_slice(
        &std::fs::read(std::env::var_os("AM_TEST_UV_CONTROL").unwrap()).unwrap(),
    )
    .unwrap();
    let package = PathBuf::from(std::env::var_os("UV_TOOL_DIR").unwrap()).join("tool-a");
    let bin = PathBuf::from(std::env::var_os("UV_TOOL_BIN_DIR").unwrap());
    assert!(
        !package.exists(),
        "Expected the original package to be moved first"
    );
    install(
        &package,
        &bin,
        control["version"].as_str().unwrap(),
        control["case"].as_str().unwrap(),
    );
    if control["case"] == "command-failure" {
        eprintln!("Controlled installer failure after writing the new package");
        std::process::exit(7);
    }
}

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    data: PathBuf,
    tools: PathBuf,
    package: PathBuf,
    bin: PathBuf,
    control: PathBuf,
    marker: PathBuf,
    sentinel: Vec<u8>,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("am-uv-result-{}-{label}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let home = root.join("home");
        let data = root.join("data");
        let tools = root.join("tools");
        let bin = home.join("bin");
        let package = home.join("uv/data/tools/tool-a");
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::copy(
            std::env::current_exe().unwrap(),
            tools.join(if cfg!(windows) { "uv.exe" } else { "uv" }),
        )
        .unwrap();
        install(&package, &bin, "1.0.0", "valid");
        let sentinel = b"original package bytes retained without automatic restoration".to_vec();
        std::fs::write(package.join("original"), &sentinel).unwrap();
        let skill = home.join(".claude/skills/tool-a");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "---\nname: tool-a\ndescription: fixture\nmetadata:\n  requires:\n    bins: [tool-a]\n---\nFixture\n").unwrap();
        Self {
            control: root.join("control.json"),
            marker: root.join("uv-executed"),
            root,
            home,
            data,
            tools,
            package,
            bin,
            sentinel,
        }
    }

    fn control(&self, version: &str, case: &str) {
        std::fs::write(
            &self.control,
            json!({"version":version,"case":case}).to_string(),
        )
        .unwrap();
    }

    fn run(&self, command: &str, target: Option<&str>) -> Value {
        let mut process = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"));
        process.arg(command).arg("--json");
        if let Some(target) = target {
            process.args([
                "--program",
                "pypi:tool-a",
                "--action",
                "upgrade",
                "--version",
                target,
            ]);
        }
        if command == "system:refresh-upstream" {
            process.args(["--only", "programs:pypi:tool-a"]);
        }
        let output = process
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("AGENT_MANAGER_DATA_DIR", &self.data)
            .env(
                "AGENT_MANAGER_USER_PATH",
                std::env::join_paths([&self.bin, &self.tools]).unwrap(),
            )
            .env("UV_TOOL_DIR", self.package.parent().unwrap())
            .env("AM_TEST_UV_CONTROL", &self.control)
            .env("AM_TEST_UV_EXECUTED", &self.marker)
            .env_remove("AGENT_MANAGER_LOCK_HELD")
            .env_remove("AGENT_MANAGER_LOCK_PID")
            .output()
            .unwrap();
        serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!(
                "{command}: {} / {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    fn backups(&self) -> Vec<PathBuf> {
        let root = self.data.join("programs/backups");
        let mut backups: Vec<_> = std::fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        backups.sort();
        backups
    }

    fn original_retained(&self) {
        let backups = self.backups();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            std::fs::read(backups[0].join("package/original")).unwrap(),
            self.sentinel
        );
        assert!(
            !self.package.join("original").exists(),
            "Unexpected automatic restoration"
        );
        assert!(self.marker.exists(), "Installer was not executed");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("Retained private uv fixture: {}", self.root.display());
        } else {
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }
}

fn retained_failure(case: &str) {
    let fixture = Fixture::new(case);
    let actual = if matches!(case, "valid" | "missing-bin") {
        "9.9.0"
    } else {
        "2.0.0"
    };
    fixture.control(actual, case);
    let preview = fixture.run("programs:preview", Some("2.0.0"));
    assert_eq!(preview["ok"], true, "{preview}");
    let notes = preview["data"]["notes"].to_string();
    assert!(notes.contains("may remain on PATH") && notes.contains("any successful action"));
    assert!(!fixture.data.exists() && !fixture.marker.exists());
    let applied = fixture.run("programs:apply", Some("2.0.0"));
    assert_eq!(applied["ok"], false, "{applied}");
    assert_eq!(applied["errors"][0]["code"], "PROGRAM_VERSION_UNVERIFIED");
    let errors = applied["errors"].to_string();
    assert!(
        errors.contains("unverified")
            && errors.contains("No automatic restoration")
            && errors.contains("newest-3"),
        "{applied}"
    );
    if actual == "2.0.0" {
        assert!(errors.contains("installed version is unknown"), "{applied}");
    }
    assert!(
        !errors.contains("装完后找不到命令"),
        "Version check must precede the entry check"
    );
    fixture.original_retained();
    assert_eq!(
        std::fs::read_to_string(fixture.package.join("declared-version")).unwrap(),
        actual
    );
    assert_eq!(fixture.run("programs:status", None)["ok"], true);
}

fn equivalent_versions() {
    for (index, (target, actual)) in [
        ("1.0", "1.0.0"),
        ("v1.0", "1.0"),
        ("1.0", "1.0+local"),
        ("1.0-1", "1.0.post1"),
    ]
    .into_iter()
    .enumerate()
    {
        let fixture = Fixture::new(&format!("equivalent-{index}"));
        fixture.control(actual, "valid");
        let applied = fixture.run("programs:apply", Some(target));
        assert_eq!(applied["ok"], true, "{target} / {actual}: {applied}");
        fixture.original_retained();
    }
}

fn invalid_target_is_zero_write() {
    let fixture = Fixture::new("invalid");
    let result = fixture.run("programs:preview", Some("invalid-version"));
    assert_eq!(result["ok"], false, "{result}");
    assert!(result["errors"]
        .to_string()
        .contains("Invalid PyPI target version"));
    assert!(!fixture.data.exists() && !fixture.marker.exists());
    assert_eq!(
        std::fs::read(fixture.package.join("original")).unwrap(),
        fixture.sentinel
    );
}

fn command_failure_still_restores_owned_original() {
    let fixture = Fixture::new("command-failure");
    fixture.control("9.9.0", "command-failure");
    let result = fixture.run("programs:apply", Some("2.0.0"));
    assert_eq!(result["ok"], false, "{result}");
    assert!(result["errors"]
        .to_string()
        .contains("Controlled installer failure"));
    assert_eq!(result["errors"][0]["code"], "PROGRAM_ERROR");
    assert_eq!(
        std::fs::read(fixture.package.join("original")).unwrap(),
        fixture.sentinel
    );
    assert_eq!(
        std::fs::read_to_string(fixture.package.join("declared-version")).unwrap(),
        "1.0.0"
    );
    assert!(fixture.bin.join(entry_name()).exists());
}

fn successful_actions_keep_the_normal_three_backup_limit() {
    let fixture = Fixture::new("retention");
    fixture.control("9.9.0", "valid");
    for _ in 0..4 {
        assert_eq!(fixture.run("programs:apply", Some("2.0.0"))["ok"], false);
    }
    let backups = fixture.backups();
    assert_eq!(backups.len(), 4);
    let original = &backups[0];
    assert_eq!(
        std::fs::read(original.join("package/original")).unwrap(),
        fixture.sentinel
    );
    fixture.control("2.0.0", "valid");
    let success = fixture.run("programs:apply", Some("2.0.0"));
    assert_eq!(success["ok"], true, "{success}");
    assert_eq!(fixture.backups().len(), 3);
    assert!(
        !original.exists(),
        "The original backup is not permanently pinned"
    );
}

fn cached_current_versions_cannot_override_actual_metadata() {
    let fixture = Fixture::new("freshness");
    std::fs::copy(
        std::env::current_exe().unwrap(),
        fixture
            .tools
            .join(if cfg!(windows) { "curl.exe" } else { "curl" }),
    )
    .unwrap();
    let refresh = fixture.run("system:refresh-upstream", None);
    assert_eq!(refresh["ok"], true, "{refresh}");
    fixture.control("9.9.0", "valid");
    assert_eq!(fixture.run("programs:apply", Some("2.0.0"))["ok"], false);
    let read = || {
        let status = fixture.run("programs:status", None);
        status["data"]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|element| element["program"] == "pypi:tool-a")
            .unwrap()
            .clone()
    };
    let current = read();
    assert_eq!(current["freshness"]["current"], "9.9.0");
    assert_eq!(current["freshness"]["state"], "ahead");
    assert_eq!(current["freshness"]["upstream"], "2.0.0");
    std::fs::remove_file(site(&fixture.package).join("tool_a-9.9.0.dist-info/METADATA")).unwrap();
    let unknown = read();
    assert!(unknown["version"].is_null() && unknown["freshness"]["current"].is_null());
    assert_eq!(unknown["freshness"]["state"], "unknown");
}

fn main() {
    let executable = std::env::current_exe().unwrap();
    if executable.file_stem().is_some_and(|name| name == "curl") {
        println!("{{\"info\":{{\"version\":\"2.0.0\"}}}}");
        return;
    }
    if executable.file_stem().is_some_and(|name| name == "uv") {
        uv_fixture(&std::env::args().skip(1).collect::<Vec<_>>());
        return;
    }
    for case in ["valid", "missing", "duplicate", "malformed", "missing-bin"] {
        retained_failure(case);
    }
    equivalent_versions();
    invalid_target_is_zero_write();
    command_failure_still_restores_owned_original();
    successful_actions_keep_the_normal_three_backup_limit();
    cached_current_versions_cannot_override_actual_metadata();
    println!("uv lifecycle: command-success version failures retained current/backup files; PEP 440 matching, early refusal, command-failure restoration, normal retention and actual-version freshness passed");
}
