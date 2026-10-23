//! Portable CLI acceptance with test-only npm/curl executables. This target uses
//! a small explicit harness so the same binary can also serve as `curl.exe` and
//! the implementation behind `npm.cmd`, without Node, Python, or a C# compiler.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
#[path = "support/native_npm_managed.rs"]
mod managed;

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
    let control = std::env::var_os("AM_NPM_MANAGED_CONTROL").map(PathBuf::from);
    let mode = control
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok());
    if let Some(path) = &control {
        std::fs::write(path.with_extension("called"), b"called").unwrap();
    }
    install_package(
        Path::new(&prefix[1]),
        package,
        if mode.as_deref() == Some("wrong-version") {
            "9.9.0"
        } else {
            version
        },
        args.iter().any(|a| a == "-g"),
    );
    let package_dir = if args.iter().any(|a| a == "-g") {
        global_modules(Path::new(&prefix[1]))
    } else {
        Path::new(&prefix[1]).join("node_modules")
    }
    .join(package);
    match mode.as_deref() {
        Some("wrong-name") => {
            let path = package_dir.join("package.json");
            let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            value["name"] = json!("another-package");
            std::fs::write(path, value.to_string()).unwrap();
        }
        Some("missing-metadata") => std::fs::remove_file(package_dir.join("package.json")).unwrap(),
        Some("malformed-metadata") => std::fs::write(package_dir.join("package.json"), b"{").unwrap(),
        Some("duplicate-metadata") => std::fs::write(package_dir.join("package.json"), format!(r#"{{"name":"{package}","version":"{version}","version":"{version}","bin":{{"tool-a":"bin/cli.js"}}}}"#)).unwrap(),
        Some("missing-entry") => std::fs::remove_file(Path::new(&prefix[1]).join("node_modules/.bin").join(command_name("tool-a"))).unwrap(),
        Some("directory-target") => {
            std::fs::remove_file(package_dir.join("bin/cli.js")).unwrap();
            std::fs::create_dir(package_dir.join("bin/cli.js")).unwrap();
        }
        Some("external-target") => {
            let external = control.as_ref().unwrap().parent().unwrap().join("external-command-target");
            std::fs::create_dir(&external).unwrap();
            executable(&external.join("cli.js"), b"#!/bin/sh\necho external\n");
            std::fs::remove_dir_all(package_dir.join("bin")).unwrap();
            link_dir(&external, &package_dir.join("bin"));
        }
        Some("comment-shim") if cfg!(windows) => std::fs::write(Path::new(&prefix[1]).join("node_modules/.bin/tool-a.cmd"), "REM \"%~dp0..\\tool-a\\bin\\cli.js\" %*\r\n").unwrap(),
        _ => {}
    }
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
            .env("AM_NPM_MANAGED_CONTROL", self.base.join("npm-control"))
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
    for version in ["99.0.0"] {
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

fn failed_installer_retains_conflicting_files() {
    let env = Env::new("recovery-conflict");
    env.skill("tool-a");
    install_package(&env.prefix, "tool-a", "1.2.0", true);
    let package = global_modules(&env.prefix).join("tool-a");
    let original = tree(&package);
    let failed = env.run(
        "programs:apply",
        &[
            "--program",
            "npm:tool-a",
            "--action",
            "upgrade",
            "--version",
            "98.0.0",
        ],
    );
    assert_eq!(failed["ok"], false, "{failed}");
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(package.join("package.json")).unwrap())
            .unwrap()["version"],
        "98.0.0",
        "recovery deleted the installer's conflicting package: {failed}"
    );
    assert!(
        failed.to_string().contains("Recovery incomplete"),
        "{failed}"
    );
    let status = env.run("programs:status", &[]);
    let backup = status["data"]["backups"][0]["id"].as_str().unwrap();
    assert_eq!(
        tree(
            &env.data
                .join("programs/backups")
                .join(backup)
                .join("package")
        ),
        original
    );
}

fn late_entry_failure_restores_current_and_retains_external_entry() {
    let env = Env::new("late-entry-failure");
    env.skill("tool-a");
    let args = [
        "--program",
        "npm:tool-a",
        "--action",
        "install",
        "--version",
        "1.2.0",
        "--entry-dir",
        "~/bin",
    ];
    let installed = env.run("programs:apply", &args);
    assert_eq!(installed["ok"], true, "{installed}");
    let entry = env.data.join("programs/bin").join(command_name("tool-a"));
    std::fs::remove_file(&entry).unwrap();
    std::fs::write(&entry, b"external entry").unwrap();
    let failed = env.run(
        "programs:apply",
        &[
            "--program",
            "npm:tool-a",
            "--action",
            "upgrade",
            "--version",
            "2.0.0",
        ],
    );
    assert_eq!(failed["ok"], false, "{failed}");
    let program = env.data.join("programs/npm/tool-a");
    assert_eq!(
        program.join("current").canonicalize().unwrap(),
        program.join("1.2.0").canonicalize().unwrap()
    );
    assert_eq!(std::fs::read(entry).unwrap(), b"external entry");
    assert!(program.join("2.0.0/.agent-manager-program.json").is_file());
}

fn restore_rejects_missing_backup_entries_and_changed_destinations_before_writes() {
    for mode in [
        "missing-entry",
        "external-entry",
        "outside-source",
        "malformed-entry",
        "directory-entry",
        "changed-package",
        "legacy-evidence",
    ] {
        let env = Env::new(mode);
        env.skill("tool-a");
        install_package(&env.prefix, "tool-a", "1.2.0", true);
        let upgraded = env.run(
            "programs:apply",
            &[
                "--program",
                "npm:tool-a",
                "--action",
                "upgrade",
                "--version",
                "2.0.0",
            ],
        );
        assert_eq!(upgraded["ok"], true, "{upgraded}");
        let backup = upgraded["data"]["backup"].as_str().unwrap();
        let dir = env.data.join("programs/backups").join(backup);
        let metadata_path = dir.join("backup.json");
        let mut metadata: Value =
            serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
        match mode {
            "missing-entry" => {
                std::fs::remove_file(metadata["links"][0]["to"].as_str().unwrap()).unwrap()
            }
            "external-entry" => {
                let path = Path::new(metadata["links"][0]["from"].as_str().unwrap());
                std::fs::remove_file(path).unwrap();
                std::fs::write(path, b"external entry").unwrap();
            }
            "outside-source" => metadata["links"][0]["to"] = json!(env.home.join("outside")),
            "malformed-entry" => metadata["links"][0] = json!({"from": "missing source"}),
            "directory-entry" => {
                let path = Path::new(metadata["links"][0]["to"].as_str().unwrap());
                std::fs::remove_file(path).unwrap();
                std::fs::create_dir(path).unwrap();
            }
            "changed-package" => std::fs::write(
                dir.join("package/externally-added"),
                b"external backup edit",
            )
            .unwrap(),
            "legacy-evidence" => {
                metadata
                    .as_object_mut()
                    .unwrap()
                    .remove("package_fingerprint");
            }
            _ => unreachable!(),
        }
        std::fs::write(&metadata_path, metadata.to_string()).unwrap();
        let actual = tree(&env.prefix);
        let saved = tree(&dir);
        for command in ["programs:preview", "programs:apply"] {
            let refused = env.run(
                command,
                &[
                    "--program",
                    "npm:tool-a",
                    "--action",
                    "restore",
                    "--backup",
                    backup,
                ],
            );
            assert_eq!(refused["ok"], false, "{mode}: {refused}");
            assert_eq!(tree(&env.prefix), actual, "{mode}: {refused}");
            assert_eq!(tree(&dir), saved, "{mode}: {refused}");
        }
    }
}

fn consumed_backup_keeps_unexpected_files_when_restoring_an_old_version() {
    let env = Env::new("restore-retained-backup");
    env.skill("tool-a");
    install_package(&env.prefix, "tool-a", "1.0.0", true);
    let mut oldest = None;
    for version in ["2.0.0", "3.0.0", "4.0.0"] {
        let result = env.run(
            "programs:apply",
            &[
                "--program",
                "npm:tool-a",
                "--action",
                "upgrade",
                "--version",
                version,
            ],
        );
        assert_eq!(result["ok"], true, "{result}");
        if oldest.is_none() {
            oldest = Some(result["data"]["backup"].as_str().unwrap().to_string());
        }
    }
    let oldest = oldest.unwrap();
    let retained = env
        .data
        .join("programs/backups")
        .join(&oldest)
        .join("external-notes");
    std::fs::write(&retained, b"keep these notes").unwrap();
    let result = env.run(
        "programs:apply",
        &[
            "--program",
            "npm:tool-a",
            "--action",
            "restore",
            "--backup",
            &oldest,
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(std::fs::read(retained).unwrap(), b"keep these notes");
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

fn entry_directory_spellings_preserve_eligibility() {
    let mut env = Env::new("entry-spelling");
    env.skill("tool-a");
    let entry = env.home.join("bin");
    let unlisted = env.home.join("not-on-path");
    let account = env.home.join(".claude/bin");
    std::fs::create_dir_all(&unlisted).unwrap();
    std::fs::create_dir_all(&account).unwrap();
    env.user_path =
        std::env::join_paths([entry.clone(), account.clone(), env.tools.clone()]).unwrap();
    let request = |command: &str, action: &str, version: &str, chosen: &str| {
        env.run(
            command,
            &[
                "--program",
                "npm:tool-a",
                "--action",
                action,
                "--version",
                version,
                "--entry-dir",
                chosen,
            ],
        )
    };
    let original = tree(&env.home);
    for chosen in [entry.to_str().unwrap(), "~/bin"] {
        let preview = request("programs:preview", "install", "1.2.0", chosen);
        assert_eq!(preview["ok"], true, "{chosen}: {preview}");
        assert!(!env.data.exists(), "preview created manager state");
        assert_eq!(tree(&env.home), original, "preview changed home files");
    }
    let rejected_paths = [unlisted.to_str().unwrap(), account.to_str().unwrap(), "bin"];
    for chosen in rejected_paths {
        let rejected = request("programs:preview", "install", "1.2.0", chosen);
        assert_eq!(rejected["ok"], false, "{chosen}: {rejected}");
        assert!(!env.data.exists(), "rejected preview created manager state");
        assert_eq!(
            tree(&env.home),
            original,
            "rejected preview changed home files"
        );
    }
    let installed = request(
        "programs:apply",
        "install",
        "1.2.0",
        entry.to_str().unwrap(),
    );
    assert_eq!(installed["ok"], true, "{installed}");
    assert!(entry.join(command_name("tool-a")).is_file());
    let status = env.run("programs:status", &[]);
    assert_eq!(status["data"]["entry_dir"], "~/bin");
    let installed_home = tree(&env.home);
    let installed_data = tree(&env.data);
    for chosen in rejected_paths {
        let rejected = request("programs:apply", "upgrade", "2.0.0", chosen);
        assert_eq!(rejected["ok"], false, "{chosen}: {rejected}");
        assert_eq!(
            tree(&env.home),
            installed_home,
            "ineligible entry changed home files"
        );
        assert_eq!(
            tree(&env.data),
            installed_data,
            "ineligible entry changed program state"
        );
    }
    for chosen in [entry.to_str().unwrap(), "~/bin"] {
        let preview = request("programs:preview", "upgrade", "2.0.0", chosen);
        assert_eq!(preview["ok"], true, "{preview}");
        assert!(
            !preview["data"]["steps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|step| step["kind"] == "entry-dir"),
            "equivalent path would rewrite the preference: {preview}"
        );
        assert_eq!(
            tree(&env.home),
            installed_home,
            "upgrade preview changed home files"
        );
        assert_eq!(
            tree(&env.data),
            installed_data,
            "upgrade preview changed program state"
        );
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
    managed::run();
    upgrade_adopt_and_restore("tool-a");
    upgrade_adopt_and_restore("@scope/tool-a");
    failed_installer_retains_conflicting_files();
    late_entry_failure_restores_current_and_retains_external_entry();
    restore_rejects_missing_backup_entries_and_changed_destinations_before_writes();
    consumed_backup_keeps_unexpected_files_when_restoring_an_old_version();
    current_pointer_is_protected();
    entry_directory_spellings_preserve_eligibility();
    probes_use_user_path();
    println!("program lifecycle: ordinary/scoped npm, rollback, adoption, current pointer, entry spellings and tool probes passed");
}
