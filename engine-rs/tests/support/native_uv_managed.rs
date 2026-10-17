//! Managed candidates must verify before completion or current-pointer writes.
use super::*;

const META: &str = ".agent-manager-program.json";

fn fixture(label: &str) -> Fixture {
    let fixture = Fixture::new(label);
    std::fs::remove_dir_all(&fixture.package).unwrap();
    std::fs::remove_file(fixture.bin.join(entry_name())).unwrap();
    std::fs::write(fixture.home.join(".claude/skills/tool-a/SKILL.md"), "---\nname: tool-a\ndescription: fixture\nmetadata:\n  openclaw:\n    requires:\n      bins: [tool-a]\n    install:\n      - kind: uv\n        package: tool-a\n        bins: [tool-a]\n---\nFixture\n").unwrap();
    fixture
}

fn program(fixture: &Fixture) -> PathBuf {
    fixture.data.join("programs/pypi/tool-a")
}

fn entry(fixture: &Fixture) -> PathBuf {
    fixture.bin.join(if cfg!(windows) {
        "tool-a.cmd"
    } else {
        "tool-a"
    })
}

fn unverified(result: &Value, stage: &str) {
    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(
        result["errors"][0]["code"], "PROGRAM_VERSION_UNVERIFIED",
        "{result}"
    );
    assert!(
        result["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains(stage),
        "{result}"
    );
}

fn first_failure(case: &str) {
    let fixture = fixture(&format!("managed-{case}"));
    let version = program(&fixture).join("2.0.0");
    fixture.control(if case == "valid" { "9.9.0" } else { "2.0.0" }, case);
    let preview = fixture.run_action("programs:preview", Some("2.0.0"), "install");
    assert_eq!(preview["ok"], true, "{preview}");
    assert!(!fixture.data.exists() && !fixture.marker.exists());
    let applied = fixture.run_action("programs:apply", Some("2.0.0"), "install");
    unverified(&applied, "before completion");
    assert!(version.join("tools/tool-a").exists());
    assert!(!version.join(META).exists());
    assert!(std::fs::symlink_metadata(program(&fixture).join("current")).is_err());
    assert!(std::fs::symlink_metadata(entry(&fixture)).is_err());
    assert!(fixture.run("programs:status", None)["data"]["entry_dir"].is_null());
    let preserved = version.join("external-notes");
    std::fs::write(&preserved, b"Do not discard a retained candidate on retry").unwrap();
    let original = std::fs::read(&preserved).unwrap();
    let package_bytes = std::fs::read(version.join("tools/tool-a/declared-version")).unwrap();
    std::fs::remove_file(&fixture.marker).unwrap();
    for command in ["programs:preview", "programs:apply"] {
        let result = fixture.run_action(command, Some("2.0.0"), "install");
        unverified(&result, "Incomplete PyPI candidate");
        assert!(
            result["errors"][0]["message"]
                .as_str()
                .unwrap()
                .contains("Incomplete PyPI candidate"),
            "{result}"
        );
        assert!(!fixture.marker.exists());
        assert_eq!(std::fs::read(&preserved).unwrap(), original);
        assert_eq!(
            std::fs::read(version.join("tools/tool-a/declared-version")).unwrap(),
            package_bytes
        );
    }
}

fn current_is_preserved() {
    let fixture = fixture("managed-current");
    fixture.control("1.0.0", "valid");
    let installed = fixture.run_action("programs:apply", Some("1.0.0"), "install");
    assert_eq!(installed["ok"], true, "{installed}");
    let current = program(&fixture).join("current");
    let original = std::fs::canonicalize(&current).unwrap();
    let original_entry = std::fs::read(entry(&fixture)).unwrap();
    let original_target = std::fs::canonicalize(entry(&fixture)).unwrap();
    fixture.control("9.9.0", "valid");
    let failed = fixture.run("programs:apply", Some("2.0.0"));
    unverified(&failed, "before completion");
    assert_eq!(std::fs::canonicalize(&current).unwrap(), original);
    assert_eq!(
        std::fs::canonicalize(entry(&fixture)).unwrap(),
        original_target
    );
    assert_eq!(std::fs::read(entry(&fixture)).unwrap(), original_entry);
    assert!(!program(&fixture).join("2.0.0").join(META).exists());
    // A previously completed alternative is freshly checked before selection and reuse.
    let alternate = program(&fixture).join("3.0.0");
    install(
        &alternate.join("tools/tool-a"),
        &alternate.join("bin"),
        "3.0.0",
        "valid",
    );
    std::fs::write(
        alternate.join(META),
        b"{\"program\":\"pypi:tool-a\",\"version\":\"3.0.0\"}",
    )
    .unwrap();
    let completion = std::fs::read(alternate.join(META)).unwrap();
    std::fs::write(
        site(&alternate.join("tools/tool-a")).join("tool_a-3.0.0.dist-info/METADATA"),
        "Metadata-Version: 2.4\nName: tool-a\nVersion: 9.9.0\n\n",
    )
    .unwrap();
    std::fs::remove_file(&fixture.marker).unwrap();
    for action in ["switch", "upgrade"] {
        let result = fixture.run_action("programs:apply", Some("3.0.0"), action);
        unverified(&result, "before current selection");
        assert!(!fixture.marker.exists());
        assert_eq!(std::fs::read(alternate.join(META)).unwrap(), completion);
        assert_eq!(std::fs::canonicalize(&current).unwrap(), original);
        assert_eq!(std::fs::read(entry(&fixture)).unwrap(), original_entry);
    }
}

fn equivalents_and_early_refusal() {
    for (index, target, actual) in [
        (0, "1.0", "1.0.0"),
        (1, "v1.0", "1.0"),
        (2, "1.0", "1.0+local"),
    ] {
        let fixture = fixture(&format!("managed-equivalent-{index}"));
        fixture.control(actual, "valid");
        let result = fixture.run_action("programs:apply", Some(target), "install");
        assert_eq!(result["ok"], true, "{result}");
        assert!(program(&fixture).join(target).join(META).is_file());
        assert_eq!(
            std::fs::canonicalize(program(&fixture).join("current")).unwrap(),
            program(&fixture).join(target)
        );
        assert!(entry(&fixture).exists());
    }
    let fixture = fixture("managed-invalid");
    for action in ["install", "switch"] {
        let result = fixture.run_action("programs:preview", Some("invalid-version"), action);
        assert_eq!(result["ok"], false, "{result}");
        assert!(
            result["errors"][0]["message"]
                .as_str()
                .unwrap()
                .contains("Invalid PyPI target version"),
            "{result}"
        );
        assert!(!fixture.data.exists() && !fixture.marker.exists());
    }
}

pub(super) fn verify() {
    for case in ["valid", "missing", "duplicate", "malformed"] {
        first_failure(case);
    }
    current_is_preserved();
    equivalents_and_early_refusal();
    println!("Managed PyPI: fresh completion/selection version evidence, retained candidates and no-execution retry refusal, unchanged prior current/entries, legacy reuse refusal, PEP 440 matching and invalid-target zero-write preview passed");
}
