//! Managed Git completion must use installed source and commit evidence.
use super::*;

const PROGRAM: &str = "git:example.invalid/owner/tool-a";
const COMMIT: &str = "1234567890abcdef1234567890abcdef12345678";

pub(super) fn evidence(package: &Path, commit: &str, version: &str, case: &str) {
    let path = site(package).join(format!("tool_a-{version}.dist-info/direct_url.json"));
    let mut direct = json!({"url": if case == "wrong-url" { "https://example.invalid/another/tool" } else { "https://example.invalid/owner/tool-a" },
        "vcs_info": {"vcs":"git", "commit_id": if case == "wrong-commit" { "abcdefabcdefabcdefabcdefabcdefabcdefabcd" } else { commit }, "requested_revision":commit}});
    if case == "lfs-metadata" {
        direct["vcs_info"]["git_lfs"] = json!(true);
    }
    if case == "invalid-origin-secret" {
        direct["vcs_info"]["git_lfs"] = json!("YOUR_TEST_PASSWORD");
    }
    if case != "missing-origin" {
        std::fs::write(&path, direct.to_string()).unwrap();
    }
    if case == "malformed-origin" {
        std::fs::write(&path, b"{").unwrap();
    }
    if case == "duplicate-origin" {
        std::fs::write(&path, format!(r#"{{"url":"https://example.invalid/another/tool","url":"https://example.invalid/owner/tool-a","vcs_info":{{"vcs":"git","commit_id":"{commit}"}}}}"#)).unwrap();
    }
    let receipt = package.join("uv-receipt.toml");
    let text = std::fs::read_to_string(&receipt)
        .unwrap()
        .lines()
        .filter(|line| !line.starts_with("requirements = "))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&receipt, text.replacen("[tool]\n", &format!("[tool]\nrequirements = [{{name = 'tool-a', git = 'https://example.invalid/owner/tool-a?rev={commit}'}}]\n"), 1)).unwrap();
    match case {
        "missing-receipt" => std::fs::remove_file(&receipt).unwrap(),
        "empty-receipt" => std::fs::write(&receipt, "[tool]\n").unwrap(),
        "invalid-receipt-secret" => std::fs::write(
            &receipt,
            "[tool]\nrequirements = []\nrequirements = ['YOUR_TEST_PASSWORD']\n",
        )
        .unwrap(),
        "invalid-identity-secret" => std::fs::write(
            site(package).join(format!("tool_a-{version}.dist-info/METADATA")),
            "Metadata-Version: 2.4\nName: tool-a\nVersion: YOUR_TEST_PASSWORD\n\n",
        )
        .unwrap(),
        "wrong-requirement" => {
            let text = std::fs::read_to_string(&receipt).unwrap();
            std::fs::write(
                &receipt,
                text.replace("name = 'tool-a'", "name = 'other-tool'"),
            )
            .unwrap();
        }
        "outside-entry" => {
            let outer = package
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("bin")
                .join(entry_name());
            std::fs::remove_file(&outer).unwrap();
            #[cfg(unix)]
            {
                let external = package
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("external-command");
                std::fs::copy(std::env::current_exe().unwrap(), &external).unwrap();
                std::os::unix::fs::symlink(external, outer).unwrap();
            }
            #[cfg(windows)]
            std::fs::write(outer, b"foreign entry bytes").unwrap();
        }
        "linked-distribution" => {
            let directory = path.parent().unwrap();
            let external = package.join("external-distribution");
            std::fs::rename(directory, &external).unwrap();
            #[cfg(unix)]
            std::os::unix::fs::symlink(external, directory).unwrap();
            #[cfg(windows)]
            junction::create(external, directory).unwrap();
        }
        "two-tools" => std::fs::create_dir(package.parent().unwrap().join("another-tool")).unwrap(),
        "foreign-provider" => {
            let text = std::fs::read_to_string(&receipt).unwrap();
            std::fs::write(
                &receipt,
                text.replace(
                    "name = 'tool-a', install-path",
                    "name = 'tool-a', from = 'other-tool', install-path",
                ),
            )
            .unwrap();
        }
        _ => {}
    }
}

fn program(fixture: &Fixture) -> PathBuf {
    fixture
        .data
        .join("programs/git/example.invalid~owner~tool-a")
}

fn fixture(case: &str) -> Fixture {
    let fixture = Fixture::new(&format!("git-{case}"));
    std::fs::remove_dir_all(&fixture.package).unwrap();
    std::fs::remove_file(fixture.bin.join(entry_name())).unwrap();
    let declared = fixture.cli(
        "programs:declare",
        &["--bin", "tool-a", "--program", PROGRAM],
    );
    assert_eq!(declared["ok"], true, "{declared}");
    fixture.control("1.0.0", case);
    fixture
}

fn origin_failures() {
    for case in [
        "wrong-url",
        "wrong-commit",
        "missing-origin",
        "duplicate-origin",
        "malformed-origin",
        "missing-receipt",
        "empty-receipt",
        "wrong-requirement",
        "linked-distribution",
        "two-tools",
        "invalid-origin-secret",
        "invalid-receipt-secret",
        "invalid-identity-secret",
    ] {
        let fixture = fixture(case);
        let result = fixture.run_action_for("programs:apply", Some(COMMIT), "install", PROGRAM);
        assert_eq!(
            result["ok"], false,
            "Git {case} candidate was selected: {result}"
        );
        assert_eq!(
            result["errors"][0]["code"], "PROGRAM_VERSION_UNVERIFIED",
            "{result}"
        );
        assert!(
            !result["errors"].to_string().contains("YOUR_TEST_PASSWORD"),
            "Metadata contents leaked: {result}"
        );
        let dir = program(&fixture);
        assert!(dir.join(COMMIT).join("tools/tool-a").is_dir());
        assert!(!dir
            .join(COMMIT)
            .join(".agent-manager-program.json")
            .exists());
        assert!(std::fs::symlink_metadata(dir.join("current")).is_err());
        let note = dir.join(COMMIT).join("external-notes");
        std::fs::write(&note, b"retain candidate").unwrap();
        std::fs::remove_file(&fixture.marker).unwrap();
        for command in ["programs:preview", "programs:apply"] {
            let retry = fixture.run_action_for(command, Some(COMMIT), "install", PROGRAM);
            assert_eq!(retry["ok"], false, "{retry}");
            assert!(
                retry["errors"]
                    .to_string()
                    .contains("Incomplete Git candidate"),
                "{retry}"
            );
            assert!(!fixture.marker.exists());
            assert_eq!(std::fs::read(&note).unwrap(), b"retain candidate");
        }
    }
}

fn entry_failures() {
    for case in ["missing-bin", "outside-entry", "foreign-provider"] {
        let fixture = fixture(case);
        let result = fixture.run_action_for("programs:apply", Some(COMMIT), "install", PROGRAM);
        assert_eq!(result["ok"], false, "{result}");
        assert_eq!(
            result["errors"][0]["code"], "PROGRAM_ENTRY_UNVERIFIED",
            "{result}"
        );
        assert!(!program(&fixture)
            .join(COMMIT)
            .join(".agent-manager-program.json")
            .exists());
        assert!(std::fs::symlink_metadata(program(&fixture).join("current")).is_err());
    }
}

fn current_selection_and_reuse() {
    const SECOND: &str = "2222222222222222222222222222222222222222";
    const THIRD: &str = "3333333333333333333333333333333333333333";
    let fixture = fixture("valid");
    let result = fixture.run_action_for("programs:apply", Some(COMMIT), "install", PROGRAM);
    assert_eq!(result["ok"], true, "{result}");
    let current = program(&fixture).join("current");
    let original = current.canonicalize().unwrap();
    fixture.control("2.0.0", "wrong-commit");
    let failed = fixture.run_action_for("programs:apply", Some(SECOND), "upgrade", PROGRAM);
    assert_eq!(
        failed["errors"][0]["code"], "PROGRAM_VERSION_UNVERIFIED",
        "{failed}"
    );
    assert_eq!(current.canonicalize().unwrap(), original);
    let alternate = program(&fixture).join(THIRD);
    install(
        &alternate.join("tools/tool-a"),
        &alternate.join("bin"),
        "3.0.0",
        "valid",
    );
    evidence(&alternate.join("tools/tool-a"), THIRD, "3.0.0", "wrong-url");
    std::fs::write(
        alternate.join(".agent-manager-program.json"),
        json!({"program":PROGRAM,"version":THIRD}).to_string(),
    )
    .unwrap();
    std::fs::remove_file(&fixture.marker).unwrap();
    for action in ["switch", "upgrade"] {
        let failed = fixture.run_action_for("programs:apply", Some(THIRD), action, PROGRAM);
        assert_eq!(
            failed["errors"][0]["code"], "PROGRAM_VERSION_UNVERIFIED",
            "{failed}"
        );
        assert_eq!(current.canonicalize().unwrap(), original);
        assert!(!fixture.marker.exists());
    }
    evidence(&alternate.join("tools/tool-a"), THIRD, "3.0.0", "valid");
    let switched = fixture.run_action_for("programs:apply", Some(THIRD), "switch", PROGRAM);
    assert_eq!(switched["ok"], true, "{switched}");
    assert_eq!(
        current.canonicalize().unwrap(),
        alternate.canonicalize().unwrap()
    );
    let reused = fixture.run_action_for("programs:apply", Some(COMMIT), "upgrade", PROGRAM);
    assert_eq!(reused["ok"], true, "{reused}");
    assert_eq!(current.canonicalize().unwrap(), original);
    assert!(!fixture.marker.exists());
}

pub(super) fn verify() {
    origin_failures();
    entry_failures();
    current_selection_and_reuse();
    let lfs = fixture("lfs-metadata");
    let valid = lfs.run_action_for("programs:apply", Some(COMMIT), "install", PROGRAM);
    assert_eq!(valid["ok"], true, "{valid}");
    let fixture = fixture("invalid-target");
    for target in ["main", "short", "12345678"] {
        let refused = fixture.run_action_for("programs:preview", Some(target), "install", PROGRAM);
        assert_eq!(refused["ok"], false, "{refused}");
        assert!(!fixture.marker.exists());
        assert!(!program(&fixture).exists());
    }
    println!("Managed Git: installed source/commit and entry gates, uncertain candidate/retry retention, stale completed reuse/switch refusal, valid controls and full-commit previews passed");
}
