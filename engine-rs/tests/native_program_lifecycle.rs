//! Portable CLI acceptance with test-only npm/curl executables. This target uses
//! a small explicit harness so the same binary can also serve as `curl.exe` and
//! the implementation behind `npm.cmd`, without Node, Python, or a C# compiler.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

fn executable(path: &Path, contents: impl AsRef<[u8]>) {
    std::fs::write(path, contents).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn link_dir(target: &Path, link: &Path) {
    // Explicit junctions make this test independent of symlink privileges.
    #[cfg(windows)]
    junction::create(target, link).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}

fn unlink_dir(link: &Path) {
    #[cfg(windows)]
    std::fs::remove_dir(link).unwrap();
    #[cfg(unix)]
    std::fs::remove_file(link).unwrap();
}

fn global_modules(prefix: &Path) -> PathBuf {
    if cfg!(windows) {
        prefix.join("node_modules")
    } else {
        prefix.join("lib/node_modules")
    }
}

fn global_bins(prefix: &Path) -> PathBuf {
    if cfg!(windows) {
        prefix.to_path_buf()
    } else {
        prefix.join("bin")
    }
}

fn command_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.cmd")
    } else {
        name.into()
    }
}

fn install_package(prefix: &Path, package: &str, version: &str, global: bool) {
    let modules = if global {
        global_modules(prefix)
    } else {
        prefix.join("node_modules")
    };
    let pkg = modules.join(package);
    std::fs::create_dir_all(pkg.join("bin")).unwrap();
    std::fs::write(
        pkg.join("package.json"),
        json!({
            "name": package, "version": version, "bin": {"tool-a": "bin/cli.js"}
        })
        .to_string(),
    )
    .unwrap();
    executable(
        &pkg.join("bin/cli.js"),
        format!("#!/bin/sh\necho {version}\n"),
    );
    let bins = if global {
        global_bins(prefix)
    } else {
        modules.join(".bin")
    };
    std::fs::create_dir_all(&bins).unwrap();
    let link = bins.join(command_name("tool-a"));
    #[cfg(windows)]
    {
        let relative = if global {
            format!("node_modules\\{package}\\bin\\cli.js")
        } else {
            format!("..\\{package}\\bin\\cli.js")
        };
        // Matches the standard npm cmd-shim target syntax.
        executable(&link, format!("@echo off\r\n\"%~dp0{relative}\" %*\r\n"));
        if global {
            let target = relative.replace('\\', "/");
            executable(
                &bins.join("tool-a.ps1"),
                format!("& node \"$basedir/{target}\" $args\r\n"),
            );
            executable(
                &bins.join("tool-a"),
                format!("#!/bin/sh\nexec node \"$basedir/{target}\" \"$@\"\n"),
            );
        }
    }
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(&link);
        let relative = if global {
            format!("../lib/node_modules/{package}/bin/cli.js")
        } else {
            format!("../{package}/bin/cli.js")
        };
        std::os::unix::fs::symlink(relative, link).unwrap();
    }
}

fn npm_fixture(args: &[String]) {
    if args.first().map(String::as_str) == Some("view") {
        assert_eq!(args.last().map(String::as_str), Some("version"));
        println!("2.0.0");
        return;
    }
    assert_eq!(args.first().map(String::as_str), Some("install"));
    let prefix = args.windows(2).find(|pair| pair[0] == "--prefix").unwrap();
    let spec = args.last().unwrap();
    let (package, version) = spec.rsplit_once('@').unwrap();
    if version == "99.0.0" {
        std::process::exit(1);
    }
    install_package(
        Path::new(&prefix[1]),
        package,
        version,
        args.iter().any(|a| a == "-g"),
    );
    if version == "98.0.0" {
        // npm can fail after it has already replaced the package and .cmd file.
        std::process::exit(1);
    }
}

struct Env {
    base: PathBuf,
    home: PathBuf,
    data: PathBuf,
    tools: PathBuf,
    prefix: PathBuf,
    user_path: std::ffi::OsString,
}

impl Env {
    fn new(tag: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "am-lifecycle-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = base.join("home");
        let data = base.join("data");
        let tools = base.join("tools with spaces");
        let prefix = home.join("npm global");
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::create_dir_all(&tools).unwrap();
        let fixture = std::env::current_exe().unwrap();
        #[cfg(windows)]
        executable(
            &tools.join("npm.cmd"),
            format!(
                "@echo off\r\n\"{}\" --npm-fixture %*\r\n",
                fixture.display()
            ),
        );
        #[cfg(unix)]
        executable(
            &tools.join("npm"),
            format!(
                "#!/bin/sh\nexec '{}' --npm-fixture \"$@\"\n",
                fixture.display()
            ),
        );
        let curl = tools.join(if cfg!(windows) { "curl.exe" } else { "curl" });
        std::fs::copy(fixture, &curl).unwrap();
        let user_path =
            std::env::join_paths([home.join("bin"), global_bins(&prefix), tools.clone()]).unwrap();
        Self {
            base,
            home,
            data,
            tools,
            prefix,
            user_path,
        }
    }

    fn run(&self, command: &str, args: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-manager-engine"))
            .arg(command)
            .args(args)
            .arg("--json")
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("AGENT_MANAGER_DATA_DIR", &self.data)
            .env("AGENT_MANAGER_USER_PATH", &self.user_path)
            // Keep the fake tools out of the process PATH: success must use the
            // explicit user PATH, and Windows must append .cmd / .exe.
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

    fn skill(&self, package: &str) {
        let dir = self.home.join(".claude/skills/tool");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), format!("---\nname: tool\ndescription: fixture\nmetadata:\n  openclaw:\n    requires:\n      bins: [tool-a]\n    install:\n      - kind: node\n        package: \"{package}\"\n        bins: [tool-a]\n---\n# tool\n")).unwrap();
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn element<'a>(status: &'a Value, id: &str) -> &'a Value {
    status["data"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["program"] == id)
        .unwrap_or_else(|| panic!("missing {id}: {status}"))
}

fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(dir: &Path, base: &Path, entries: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.symlink_metadata().unwrap().file_type().is_symlink() {
                entries.push((
                    path.strip_prefix(base).unwrap().to_path_buf(),
                    std::fs::read_link(&path)
                        .unwrap()
                        .to_string_lossy()
                        .as_bytes()
                        .to_vec(),
                ));
            } else if path.is_dir() {
                walk(&path, base, entries);
            } else {
                entries.push((
                    path.strip_prefix(base).unwrap().to_path_buf(),
                    std::fs::read(path).unwrap(),
                ));
            }
        }
    }
    let mut entries = Vec::new();
    walk(dir, dir, &mut entries);
    entries.sort();
    entries
}

fn upgrade_adopt_and_restore(package: &str) {
    let env = Env::new("npm");
    env.skill(package);
    install_package(&env.prefix, package, "1.2.0", true);
    let original = tree(&env.prefix);
    let id = format!("npm:{package}");
    let preview = env.run(
        "programs:preview",
        &[
            "--program",
            &id,
            "--action",
            "upgrade",
            "--version",
            "2.0.0",
        ],
    );
    assert_eq!(preview["ok"], true, "{preview}");
    assert_eq!(
        tree(&env.prefix),
        original,
        "preview changed the original installation"
    );
    for version in ["99.0.0", "98.0.0"] {
        let failed = env.run(
            "programs:apply",
            &[
                "--program",
                &id,
                "--action",
                "upgrade",
                "--version",
                version,
            ],
        );
        assert_eq!(failed["ok"], false, "{failed}");
        assert_eq!(
            tree(&env.prefix),
            original,
            "failed upgrade did not restore all files: {failed}"
        );
        assert!(
            !env.home.join("node_modules").exists(),
            "installed in the prefix's parent"
        );
    }
    let upgraded = env.run(
        "programs:apply",
        &[
            "--program",
            &id,
            "--action",
            "upgrade",
            "--version",
            "2.0.0",
        ],
    );
    assert_eq!(upgraded["ok"], true, "{upgraded}");
    let status = env.run("programs:status", &[]);
    assert_eq!(element(&status, &id)["version"], "2.0.0", "{status}");
    let backup = status["data"]["backups"][0]["id"].as_str().unwrap();
    let restored = env.run(
        "programs:apply",
        &["--program", &id, "--action", "restore", "--backup", backup],
    );
    assert_eq!(restored["ok"], true, "{restored}");
    assert_eq!(tree(&env.prefix), original);
    let adopted = env.run(
        "programs:apply",
        &[
            "--program",
            &id,
            "--action",
            "adopt",
            "--entry-dir",
            "~/bin",
        ],
    );
    assert_eq!(adopted["ok"], true, "{adopted}");
    assert!(!global_modules(&env.prefix).join(package).exists());
    assert!(!global_bins(&env.prefix)
        .join(command_name("tool-a"))
        .exists());
    let status = env.run("programs:status", &[]);
    assert_eq!(
        element(&status, &id)["managed_current"],
        "1.2.0",
        "{status}"
    );
    let backup = status["data"]["backups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["reason"] == "adopt")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let undone = env.run(
        "programs:apply",
        &["--program", &id, "--action", "unadopt", "--backup", backup],
    );
    assert_eq!(undone["ok"], true, "{undone}");
    assert_eq!(tree(&env.prefix), original);
    assert!(!env.home.join("bin").join(command_name("tool-a")).exists());
    assert_eq!(env.run("programs:status", &[])["data"]["links"], json!({}));
}

fn current_pointer_is_protected() {
    let env = Env::new("current");
    let dir = env.data.join("programs/npm/tool-a");
    for version in ["1.2.0", "1.3.0"] {
        std::fs::create_dir_all(dir.join(version)).unwrap();
        std::fs::write(dir.join(version).join(".agent-manager-program.json"), "{}").unwrap();
    }
    let current = dir.join("current");
    link_dir(&dir.join("1.2.0"), &current);
    let status = env.run("programs:status", &[]);
    assert_eq!(element(&status, "npm:tool-a")["managed_current"], "1.2.0");
    assert!(!element(&status, "npm:tool-a")["managed_versions"]
        .as_array()
        .unwrap()
        .contains(&json!("current")));
    for version in ["1.2.0", "..", ".", "current"] {
        let result = env.run(
            "programs:apply",
            &[
                "--program",
                "npm:tool-a",
                "--action",
                "remove",
                "--version",
                version,
            ],
        );
        assert_eq!(result["ok"], false, "{result}");
        assert!(current.join(".agent-manager-program.json").is_file());
    }
    let result = env.run(
        "programs:apply",
        &[
            "--program",
            "npm:tool-a",
            "--action",
            "remove",
            "--version",
            "1.3.0",
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
    assert!(!dir.join("1.3.0").exists());
    unlink_dir(&current);
    let outside = env.base.join("outside/1.2.0");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join(".agent-manager-program.json"), "{}").unwrap();
    for target in [Some(outside), Some(dir.join("missing")), None] {
        if let Some(target) = &target {
            link_dir(target, &current);
        }
        let result = env.run(
            "programs:apply",
            &[
                "--program",
                "npm:tool-a",
                "--action",
                "remove",
                "--version",
                "1.2.0",
            ],
        );
        assert_eq!(result["ok"], false, "{result}");
        assert!(dir.join("1.2.0/.agent-manager-program.json").is_file());
        if target.is_some() {
            unlink_dir(&current);
        }
    }
}

fn probes_use_user_path() {
    let env = Env::new("probes");
    env.skill("tool-a");
    install_package(&env.prefix, "tool-a", "1.2.0", true);
    let result = env.run(
        "system:refresh-upstream",
        &["--only", "programs:npm:tool-a"],
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(
        result["data"]["elements"][0]["upstream"], "2.0.0",
        "{result}"
    );
    let env = Env::new("pypi");
    env.skill("fixture");
    let skill = env.home.join(".claude/skills/tool/SKILL.md");
    let text = std::fs::read_to_string(&skill)
        .unwrap()
        .replace("kind: node", "kind: uv");
    std::fs::write(skill, text).unwrap();
    // A PyPI install preview queries the version but never runs uv or installs.
    executable(
        &env.tools.join(command_name("uv")),
        if cfg!(windows) {
            "@exit /b 91\r\n"
        } else {
            "#!/bin/sh\nexit 91\n"
        },
    );
    let result = env.run(
        "programs:preview",
        &[
            "--program",
            "pypi:fixture",
            "--action",
            "install",
            "--entry-dir",
            "~/bin",
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["data"]["target_version"], "3.4.5", "{result}");
    let managed = env.data.join("programs/pypi/fixture");
    std::fs::create_dir_all(managed.join("1.0.0")).unwrap();
    std::fs::write(managed.join("1.0.0/.agent-manager-program.json"), "{}").unwrap();
    link_dir(&managed.join("1.0.0"), &managed.join("current"));
    let result = env.run(
        "system:refresh-upstream",
        &["--only", "programs:pypi:fixture"],
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(
        result["data"]["elements"][0]["upstream"], "3.4.5",
        "{result}"
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--npm-fixture") {
        npm_fixture(&args[1..]);
        return;
    }
    if std::env::current_exe()
        .unwrap()
        .file_stem()
        .is_some_and(|name| name == "curl")
    {
        assert_eq!(args.first().map(String::as_str), Some("-fsSL"));
        assert!(args.last().unwrap().starts_with("https://pypi.org/pypi/"));
        println!("{{\"info\":{{\"version\":\"3.4.5\"}}}}");
        return;
    }
    // Panics retain a nonzero exit status, including when Cargo supplies a test
    // filter. No fixture processes or network calls are required outside here.
    upgrade_adopt_and_restore("tool-a");
    upgrade_adopt_and_restore("@scope/tool-a");
    current_pointer_is_protected();
    probes_use_user_path();
    println!("program lifecycle: ordinary/scoped npm, rollback, adoption, current pointer, and tool probes passed");
}
