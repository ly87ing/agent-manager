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
