//! Successful npm exit is insufficient before completion and selection.
use super::*;

const META: &str = ".agent-manager-program.json";

fn action(env: &Env, command: &str, action: &str, version: &str) -> Value {
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
            "~/bin",
        ],
    )
}

pub(super) fn run() {
    for case in [
        "wrong-version",
        "wrong-name",
        "missing-metadata",
        "malformed-metadata",
        "duplicate-metadata",
        "missing-entry",
        "directory-target",
        "external-target",
    ] {
        failure(case);
    }
    #[cfg(windows)]
    failure("comment-shim");
    completed_reuse_and_staging_are_not_overwritten();
    standalone_links_recheck_current();
}

fn standalone_links_recheck_current() {
    for case in ["wrong-version", "missing-target", "missing-entry"] {
        let mut env = Env::new("npm-standalone-link");
        env.skill("tool-a");
        let installed = action(&env, "programs:apply", "install", "1.0.0");
        assert_eq!(installed["ok"], true, "{installed}");
        for operation in ["unlink", "link", "unlink"] {
            let control = action(&env, "programs:apply", operation, "1.0.0");
            assert_eq!(control["ok"], true, "{control}");
        }
        let pdir = env.data.join("programs/npm/tool-a");
        let candidate = pdir.join("1.0.0");
        let shim = env.data.join("programs/bin").join(command_name("tool-a"));
        std::fs::remove_file(&shim).unwrap();
        let package = candidate.join("node_modules/tool-a");
        match case {
            "wrong-version" => {
                let file = package.join("package.json");
                let mut metadata: Value =
                    serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
                metadata["version"] = json!("9.9.0");
                std::fs::write(file, metadata.to_string()).unwrap();
            }
            "missing-target" => std::fs::remove_file(package.join("bin/cli.js")).unwrap(),
            "missing-entry" => std::fs::remove_file(
                candidate
                    .join("node_modules/.bin")
                    .join(command_name("tool-a")),
            )
            .unwrap(),
            _ => unreachable!(),
        }
        let current = pdir.join("current").canonicalize().unwrap();
        let retained = tree(&candidate);
        let alternate = env.home.join("alternate-bin");
        std::fs::create_dir(&alternate).unwrap();
        let mut paths = std::env::split_paths(&env.user_path).collect::<Vec<_>>();
        paths.push(alternate.clone());
        env.user_path = std::env::join_paths(paths).unwrap();
        let failed = env.run(
            "programs:apply",
            &[
                "--program",
                "npm:tool-a",
                "--action",
                "link",
                "--entry-dir",
                alternate.to_str().unwrap(),
            ],
        );
        assert_eq!(
            failed["ok"], false,
            "Standalone {case} link was accepted: {failed}"
        );
        assert_eq!(
            failed["errors"][0]["code"],
            if case == "wrong-version" {
                "PROGRAM_VERSION_UNVERIFIED"
            } else {
                "PROGRAM_ENTRY_UNVERIFIED"
            },
            "{failed}"
        );
        assert!(std::fs::symlink_metadata(&shim).is_err());
        assert_eq!(std::fs::read_dir(&alternate).unwrap().count(), 0);
        assert_eq!(pdir.join("current").canonicalize().unwrap(), current);
        assert_eq!(tree(&candidate), retained);
        let status = env.run("programs:status", &[]);
        assert_eq!(
            status["data"]["entry_dir"], "~/bin",
            "Failed link saved its directory: {status}"
        );
    }
}

fn failure(case: &str) {
    let env = Env::new("npm-managed-version");
    env.skill("tool-a");
    let pdir = env.data.join("programs/npm/tool-a");
    let installed = action(&env, "programs:apply", "install", "1.0.0");
    assert_eq!(installed["ok"], true, "{installed}");
    let current = std::fs::canonicalize(pdir.join("current")).unwrap();
    let before = tree(&env.home);
    std::fs::write(env.base.join("npm-control"), case).unwrap();
    let preview = action(&env, "programs:preview", "upgrade", "2.0.0");
    assert_eq!(preview["ok"], true, "{preview}");
    assert!(!pdir.join("2.0.0").exists());
    let result = action(&env, "programs:apply", "upgrade", "2.0.0");
    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(
        result["errors"][0]["code"],
        if matches!(
            case,
            "missing-entry" | "directory-target" | "external-target" | "comment-shim"
        ) {
            "PROGRAM_ENTRY_UNVERIFIED"
        } else {
            "PROGRAM_VERSION_UNVERIFIED"
        },
        "{result}"
    );
    assert!(!pdir.join("2.0.0").join(META).exists());
    assert_eq!(
        std::fs::canonicalize(pdir.join("current")).unwrap(),
        current
    );
    assert_eq!(tree(&env.home), before);
    assert!(pdir.join("2.0.0").is_dir());
    let retained = tree(&pdir.join("2.0.0"));
    let called = env.base.join("npm-control.called");
    std::fs::remove_file(&called).unwrap();
    for command in ["programs:preview", "programs:apply"] {
        let retry = action(&env, command, "upgrade", "2.0.0");
        assert_eq!(retry["ok"], false, "{retry}");
        assert!(!called.exists());
        assert_eq!(tree(&pdir.join("2.0.0")), retained);
        assert_eq!(
            std::fs::canonicalize(pdir.join("current")).unwrap(),
            current
        );
        assert_eq!(tree(&env.home), before);
    }
}

fn completed_reuse_and_staging_are_not_overwritten() {
    let env = Env::new("npm-completed");
    env.skill("tool-a");
    for version in ["1.0.0", "2.0.0"] {
        let result = action(&env, "programs:apply", "install", version);
        assert_eq!(result["ok"], true, "{result}");
    }
    let result = action(&env, "programs:apply", "switch", "1.0.0");
    assert_eq!(result["ok"], true, "{result}");
    let pdir = env.data.join("programs/npm/tool-a");
    let current = std::fs::canonicalize(pdir.join("current")).unwrap();
    let bytes = tree(&env.home);
    let package = pdir.join("2.0.0/node_modules/tool-a");
    std::fs::rename(package.join("bin"), package.join("internal-bin")).unwrap();
    link_dir(&package.join("internal-bin"), &package.join("bin"));
    for version in ["2.0.0", "1.0.0"] {
        let result = action(&env, "programs:apply", "switch", version);
        assert_eq!(
            result["ok"], true,
            "Internal candidate target refused: {result}"
        );
    }
    let metadata = package.join("package.json");
    let original = std::fs::read(&metadata).unwrap();
    let mut changed: Value = serde_json::from_slice(&original).unwrap();
    changed["version"] = json!("9.9.0");
    std::fs::write(&metadata, changed.to_string()).unwrap();
    let called = env.base.join("npm-control.called");
    std::fs::remove_file(&called).unwrap();
    for value in ["switch", "upgrade"] {
        let result = action(&env, "programs:apply", value, "2.0.0");
        assert_eq!(
            result["errors"][0]["code"], "PROGRAM_VERSION_UNVERIFIED",
            "{result}"
        );
        assert!(!called.exists());
        assert_eq!(
            std::fs::canonicalize(pdir.join("current")).unwrap(),
            current
        );
    }
    std::fs::write(metadata, original).unwrap();
    std::fs::remove_file(
        pdir.join("2.0.0/node_modules/.bin")
            .join(command_name("tool-a")),
    )
    .unwrap();
    let retained = tree(&pdir.join("2.0.0"));
    for value in ["switch", "upgrade"] {
        let result = action(&env, "programs:apply", value, "2.0.0");
        assert_eq!(
            result["errors"][0]["code"], "PROGRAM_ENTRY_UNVERIFIED",
            "{result}"
        );
        assert!(!called.exists());
        assert_eq!(
            std::fs::canonicalize(pdir.join("current")).unwrap(),
            current
        );
        assert_eq!(tree(&pdir.join("2.0.0")), retained);
        assert_eq!(tree(&env.home), bytes);
    }
    let staging = pdir.join(".installing-3.0.0");
    std::fs::create_dir(&staging).unwrap();
    std::fs::write(staging.join("external-notes"), b"preserve me").unwrap();
    let original = tree(&staging);
    for command in ["programs:preview", "programs:apply"] {
        let result = action(&env, command, "upgrade", "3.0.0");
        assert_eq!(
            result["errors"][0]["code"], "PROGRAM_VERSION_UNVERIFIED",
            "{result}"
        );
        assert!(!called.exists());
        assert_eq!(tree(&staging), original);
        assert!(!pdir.join("3.0.0").exists());
    }
}
