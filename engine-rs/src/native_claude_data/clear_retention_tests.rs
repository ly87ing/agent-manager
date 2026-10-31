//! Project preservation and independent retained-content readback regressions.
use super::tests::fixture;
use super::*;

const ORCA_MARKER: &[u8] = br#"{"owner":"stablyai/orca/claude-agent-view","setup":"YOUR_SETUP","binding":{"context":"YOUR_CONTEXT","chord":"YOUR_CHORD"}}"#;

#[test]
fn confirmed_cleanup_removes_orca_claude_marker_and_shortcuts_only_in_default_scope() {
    let (path, ctx) = fixture();
    let root = ctx.home_dir.join(".claude");
    let marker = ORCA_MARKER;
    let shortcuts = br#"{"bindings":[{"context":"YOUR_CONTEXT","bindings":{"YOUR_CHORD":"command:orca-agent-view"}}]}"#;
    std::fs::write(root.join("orca-agent-view.json"), marker).unwrap();
    std::fs::write(root.join("keybindings.json"), shortcuts).unwrap();
    let protected = [
        (
            ".codex/config.toml",
            b"PRIVATE_CODEX_CONFIGURATION".as_slice(),
        ),
        (".codex/auth.json", b"PRIVATE_CODEX_CREDENTIAL".as_slice()),
        (".zshrc", b"YOUR_SHARED_STARTUP".as_slice()),
        ("orca/other-profile/orca-agent-view.json", marker),
        (
            "orca/preferences.json",
            b"YOUR_OTHER_CONFIGURATION".as_slice(),
        ),
    ];
    for (name, value) in protected {
        let file = ctx.home_dir.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, value).unwrap();
    }
    let snapshot = Snapshot::capture(&ctx, true).unwrap();
    let preview = snapshot.data();
    let canonical_root = std::fs::canonicalize(&root).unwrap();
    for name in ["orca-agent-view.json", "keybindings.json"] {
        assert_eq!(
            preview["configuration_paths"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|value| **value == json!(canonical_root.join(name)))
                .count(),
            1
        );
    }
    assert_eq!(
        std::fs::read(root.join("orca-agent-view.json")).unwrap(),
        marker
    );
    assert_eq!(
        std::fs::read(root.join("keybindings.json")).unwrap(),
        shortcuts
    );
    assert!(!ctx.data_dir.exists());
    let result = execute(&ctx, &snapshot.base, true, "orca-marker-fixture").unwrap();
    assert_eq!(result["status"], "complete", "{result}");
    assert_eq!(result["scope_verified"], true);
    assert!(!root.join("orca-agent-view.json").exists());
    assert!(!root.join("keybindings.json").exists());
    for (name, value) in protected {
        assert_eq!(
            std::fs::read(ctx.home_dir.join(name)).unwrap(),
            value,
            "{name}"
        );
    }
    assert!(!result.to_string().contains("PRIVATE_"));
    drop(snapshot);
    std::fs::remove_dir_all(path).unwrap();
}

#[cfg(unix)]
#[test]
fn orca_marker_aliases_and_unclassified_siblings_still_refuse_before_writes() {
    for alias in ["symlink", "hardlink", "unknown_sibling"] {
        let (path, ctx) = fixture();
        let root = ctx.home_dir.join(".claude");
        let external = path.join("external-marker.json");
        std::fs::write(&external, ORCA_MARKER).unwrap();
        let marker = root.join("orca-agent-view.json");
        match alias {
            "symlink" => std::os::unix::fs::symlink(&external, &marker).unwrap(),
            "hardlink" => std::fs::hard_link(&external, &marker).unwrap(),
            _ => {
                std::fs::write(&marker, ORCA_MARKER).unwrap();
                std::fs::write(root.join("PERSONAL.md"), b"YOUR_DOCUMENT").unwrap();
            }
        }
        assert!(Snapshot::capture(&ctx, true).is_err(), "{alias}");
        assert_eq!(std::fs::read(&external).unwrap(), ORCA_MARKER);
        assert!(root.join("settings.json").exists());
        assert!(!ctx.data_dir.exists());
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn unverified_or_changed_orca_markers_refuse_without_writes() {
    for value in [
        br#"{"owner":"YOUR_OTHER_OWNER"}"#.to_vec(),
        br#"{"setup":"YOUR_SETUP"}"#.to_vec(),
        br#"{"owner":"YOUR_OTHER_OWNER","owner":"stablyai/orca/claude-agent-view"}"#.to_vec(),
        b"INVALID_JSON".to_vec(),
        b"[]".to_vec(),
        br#"["stablyai/orca/claude-agent-view"]"#.to_vec(),
        vec![b' '; 64 * 1024 + 1],
    ] {
        let (path, ctx) = fixture();
        let marker = ctx.home_dir.join(".claude/orca-agent-view.json");
        std::fs::write(&marker, &value).unwrap();
        assert!(Snapshot::capture(&ctx, true).is_err());
        assert_eq!(std::fs::read(&marker).unwrap(), value);
        assert!(!ctx.data_dir.exists());
        std::fs::remove_dir_all(path).unwrap();
    }
    let (path, ctx) = fixture();
    let marker = ctx.home_dir.join(".claude/orca-agent-view.json");
    std::fs::write(&marker, ORCA_MARKER).unwrap();
    let snapshot = Snapshot::capture(&ctx, true).unwrap();
    let changed = br#"{"owner":"stablyai/orca/claude-agent-view","setup":"YOUR_NEW_SETUP"}"#;
    std::fs::write(&marker, changed).unwrap();
    assert!(execute(&ctx, &snapshot.base, true, "stale-orca-marker").is_err());
    assert_eq!(std::fs::read(&marker).unwrap(), changed);
    assert!(ctx.home_dir.join(".claude/settings.json").exists());
    assert!(!ctx.data_dir.exists());
    drop(snapshot);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn confirmed_cleanup_removes_all_default_configuration_including_telemetry() {
    let (path, ctx) = fixture();
    let root = ctx.home_dir.join(".claude");
    let names = [
        "settings.json",
        "settings.local.json",
        "managed-settings.json",
        ".claude.json",
        "remote-settings.json",
    ];
    let external = path.join("external");
    std::fs::create_dir(&external).unwrap();
    std::fs::write(
        external.join("certificate.pem"),
        b"PRIVATE_EXTERNAL_CERTIFICATE",
    )
    .unwrap();
    std::fs::write(external.join("helper.sh"), b"PRIVATE_EXTERNAL_HELPER").unwrap();
    for (index, name) in names.iter().enumerate() {
        let bytes = match index {
            1 => "INVALID_PRIVATE_JSON".to_owned(),
            2 => r#"{"env":{},"env":{"OTEL_UNKNOWN":"PRIVATE_VALUE"}}"#.to_owned(),
            _ => json!({"env":{"DISABLE_TELEMETRY":"1","OTEL_UNKNOWN":"PRIVATE_VALUE","OTEL_EXPORTER_OTLP_CERTIFICATE":external.join("certificate.pem")},"otelHeadersHelper":external.join("helper.sh"),"unknownTelemetry":true}).to_string(),
        };
        std::fs::write(root.join(name), bytes).unwrap();
    }
    std::fs::write(ctx.home_dir.join(".claude.json"), b"INVALID_PRIVATE_JSON").unwrap();
    let snapshot = Snapshot::capture(&ctx, true).unwrap();
    let result = execute(&ctx, &snapshot.base, true, "remove-telemetry-fixture").unwrap();
    assert_eq!(result["status"], "complete", "{result}");
    assert_eq!(result["telemetry_policy"], "remove");
    assert_eq!(result["scope_verified"], true);
    for name in names {
        assert!(!root.join(name).exists(), "{name}");
    }
    assert!(!ctx.home_dir.join(".claude.json").exists());
    assert!(!result.to_string().contains("PRIVATE_"));
    assert_eq!(
        std::fs::read(ctx.home_dir.join("project/.claude/settings.json")).unwrap(),
        b"PROJECT_SOURCE"
    );
    assert_eq!(
        std::fs::read(external.join("certificate.pem")).unwrap(),
        b"PRIVATE_EXTERNAL_CERTIFICATE"
    );
    assert_eq!(
        std::fs::read(external.join("helper.sh")).unwrap(),
        b"PRIVATE_EXTERNAL_HELPER"
    );
    drop(snapshot);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn new_project_or_unknown_root_content_stops_before_configuration_deletion() {
    for name in [".git", "PERSONAL.md"] {
        let (path, ctx) = fixture();
        let snapshot = Snapshot::capture(&ctx, true).unwrap();
        let mut progress = Progress::default();
        let result = apply(
            &snapshot,
            &mut progress,
            || Ok(()),
            |phase, _| {
                if phase == "configurations" {
                    std::fs::write(
                        ctx.home_dir.join(".claude").join(name),
                        b"PERSONAL_PROJECT_DATA",
                    )?;
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(progress.files, 0);
        assert!(ctx.home_dir.join(".claude/settings.json").exists());
        assert!(ctx.home_dir.join(".claude.json").exists());
        drop(snapshot);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn cleanup_retains_projects_worktrees_and_unclassified_sources() {
    let (path, ctx) = fixture();
    let root = ctx.home_dir.join(".claude");
    let retained = [
        ("worktrees/without-vcs/main.rs", "UNCOMMITTED_WORKTREE"),
        ("skills/personal/.git", "gitdir: YOUR_EXTERNAL_GIT_DIR"),
        ("skills/personal/README.md", "PERSONAL_PROJECT_DOC"),
        ("unclassified-source/main.py", "PERSONAL_SOURCE"),
        ("unclassified-source/README.md", "PERSONAL_DOC"),
        ("projects/project/src/main.rs", "SOURCE_WITHOUT_VCS"),
        (
            "file-history/personal-project/main.py",
            "PERSONAL_SOURCE_IN_RECOVERY_ROOT",
        ),
        ("projects/project/11111111-1111-4111-8111-111111111111/tool-results/personal-source/main.rs", "SOURCE_IN_TOOL_RESULTS"),
        ("uploads/11111111-1111-4111-8111-111111111111/personal-source/main.py", "SOURCE_IN_UPLOADS"),
        ("image-cache/11111111-1111-4111-8111-111111111111/personal-source/README.md", "DOC_IN_IMAGE_CACHE"),
    ];
    for (name, bytes) in retained {
        let file = root.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }
    let snapshot = Snapshot::capture(&ctx, true).unwrap();
    let result = execute(&ctx, &snapshot.base, true, "protected-fixture").unwrap();
    for (name, bytes) in retained {
        assert_eq!(std::fs::read(root.join(name)).unwrap(), bytes.as_bytes());
    }
    assert!(!root
        .join("projects/project/11111111-1111-4111-8111-111111111111.jsonl")
        .exists());
    assert!(!ctx.home_dir.join(".claude.json").exists());
    assert_ne!(
        result["status"], "complete",
        "Unknown ownership must remain visible"
    );
    drop(snapshot);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn a_project_marker_added_before_leaf_deletion_preserves_the_leaf() {
    let (path, ctx) = fixture();
    let snapshot = Snapshot::capture(&ctx, true).unwrap();
    let mut progress = Progress::default();
    let result = apply(
        &snapshot,
        &mut progress,
        || Ok(()),
        |phase, _| {
            if phase == "files" {
                std::fs::write(
                    ctx.home_dir.join(".claude/projects/project/.git"),
                    "gitdir: YOUR_GIT_DIR",
                )?;
            }
            Ok(())
        },
    );
    assert!(result.is_err());
    assert_eq!(
        std::fs::read(
            ctx.home_dir
                .join(".claude/projects/project/11111111-1111-4111-8111-111111111111.jsonl")
        )
        .unwrap(),
        b"PRIVATE_SESSION"
    );
    drop(snapshot);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn retained_readback_reflects_new_and_removed_unknown_content() {
    for initially_present in [false, true] {
        let (path, ctx) = fixture();
        let unknown = ctx.home_dir.join(".claude/personal-source");
        if initially_present {
            std::fs::create_dir(&unknown).unwrap();
            std::fs::write(unknown.join("README.md"), "PERSONAL_DOC").unwrap();
        }
        let snapshot = Snapshot::capture(&ctx, true).unwrap();
        let mut progress = Progress::default();
        apply(&snapshot, &mut progress, || Ok(()), |_, _| Ok(())).unwrap();
        if initially_present {
            std::fs::remove_dir_all(&unknown).unwrap();
        } else {
            std::fs::create_dir(&unknown).unwrap();
            std::fs::write(unknown.join("README.md"), "PERSONAL_DOC").unwrap();
        }
        let after = readback(&ctx, &snapshot).unwrap();
        assert_eq!(after.retained.is_empty(), initially_present);
        assert_eq!(after.issue.is_none(), initially_present);
        let data = progress.data(
            &snapshot,
            if after.issue.is_none() {
                "complete"
            } else {
                "partial"
            },
            true,
            "fixture",
            after.retained,
            true,
        );
        assert_eq!(data["retained_verified"], true);
        assert_eq!(
            data["retained"].as_array().unwrap().is_empty(),
            initially_present
        );
        drop(snapshot);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn directory_changes_recheck_unknown_files_before_selecting_another_leaf() {
    let (path, ctx) = fixture();
    let root =
        Dir::open_ambient_dir(ctx.home_dir.join(".claude"), cap_std::ambient_authority()).unwrap();
    let project = root.open_dir("projects/project").unwrap();
    let mut observed = std::collections::BTreeMap::new();
    inspect_directory(&project, Path::new("projects/project"), &mut observed).unwrap();
    project.write("main.rs", "NEW_PERSONAL_SOURCE").unwrap();
    assert!(inspect_directory(&project, Path::new("projects/project"), &mut observed).is_err());
    assert_eq!(project.read("main.rs").unwrap(), b"NEW_PERSONAL_SOURCE");
    drop(project);
    drop(root);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn a_project_root_and_unclassified_root_file_refuse_before_writes() {
    for marker in [".git", ".hg", ".svn", "Cargo.toml", "PERSONAL.md"] {
        let (path, ctx) = fixture();
        let file = ctx.home_dir.join(".claude").join(marker);
        std::fs::write(&file, "PERSONAL_PROJECT_DATA").unwrap();
        assert!(Snapshot::capture(&ctx, true).is_err(), "{marker}");
        assert_eq!(std::fs::read(&file).unwrap(), b"PERSONAL_PROJECT_DATA");
        assert!(ctx
            .home_dir
            .join(".claude/projects/project/11111111-1111-4111-8111-111111111111.jsonl")
            .exists());
        assert!(!ctx.data_dir.exists());
        std::fs::remove_dir_all(path).unwrap();
    }
}
