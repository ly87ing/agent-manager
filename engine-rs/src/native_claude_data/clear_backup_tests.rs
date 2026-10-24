//! Exact configuration backups share the default cleanup's evidence and guards.
use super::tests::fixture;
use super::*;

#[test]
fn recognized_configuration_backups_are_listed_counted_and_removed() {
    let (path, ctx) = fixture();
    let files = [
        ".claude/backups/.claude.json.backup.1770000000000",
        ".claude/backups/.claude.json.corrupted.1770000000001",
        ".claude.json.backup.1770000000002",
        ".claude.json.backup.1770000000003",
    ];
    std::fs::create_dir_all(ctx.home_dir.join(".claude/backups")).unwrap();
    for name in files {
        std::fs::write(ctx.home_dir.join(name), b"PRIVATE_CONFIGURATION_BACKUP").unwrap();
    }
    let document = ctx.home_dir.join("PERSONAL_DOCUMENT.md");
    std::fs::write(&document, b"PERSONAL_DOCUMENT").unwrap();
    let snapshot = Snapshot::capture(&ctx, true).unwrap();
    let preview = snapshot.data();
    for name in files {
        let expected = dunce::canonicalize(ctx.home_dir.join(name)).unwrap();
        assert!(preview["configuration_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == &json!(expected)));
    }
    let selected_files: u64 = preview["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|group| group["files"].as_u64().unwrap())
        .sum();
    assert_eq!(selected_files, 9);
    let result = execute(&ctx, &snapshot.base, true, "backups-fixture").unwrap();
    assert_eq!(result["status"], "complete", "{result}");
    assert_eq!(result["deleted_files"], selected_files);
    assert_eq!(result["scope_verified"], true);
    for name in files {
        assert!(!ctx.home_dir.join(name).exists());
    }
    assert_eq!(std::fs::read(document).unwrap(), b"PERSONAL_DOCUMENT");
    assert_eq!(
        std::fs::read(ctx.home_dir.join("project/.claude/settings.json")).unwrap(),
        b"PROJECT_SOURCE"
    );
    assert!(!preview.to_string().contains("PRIVATE_"));
    drop(snapshot);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn a_home_backup_without_a_default_directory_is_still_selected() {
    let work = std::env::temp_dir().join(format!("am-claude-home-backup-{}", store::uuid_v4()));
    let ctx = Context {
        home_dir: work.join("home"),
        data_dir: work.join("manager"),
    };
    std::fs::create_dir_all(&ctx.home_dir).unwrap();
    let file = ctx.home_dir.join(".claude.json.backup.1770000000000");
    std::fs::write(&file, b"PRIVATE_CONFIGURATION_BACKUP").unwrap();
    let snapshot = Snapshot::capture(&ctx, true).unwrap();
    assert_eq!(snapshot.data()["has_changes"], true);
    assert!(!ctx.data_dir.exists());
    let result = execute(&ctx, &snapshot.base, true, "home-backup-fixture").unwrap();
    assert_eq!(result["status"], "complete");
    assert_eq!(result["deleted_files"], 1);
    assert!(!file.exists());
    drop(snapshot);
    std::fs::remove_dir_all(work).unwrap();
}

#[test]
fn backup_directories_with_unknown_documents_remain_protected() {
    let (path, ctx) = fixture();
    let dir = ctx.home_dir.join(".claude/backups");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(
        dir.join(".claude.json.backup.1770000000000"),
        b"PRIVATE_BACKUP",
    )
    .unwrap();
    std::fs::write(dir.join("README.md"), b"PERSONAL_DOCUMENT").unwrap();
    let snapshot = Snapshot::capture(&ctx, true).unwrap();
    let preview = snapshot.data();
    assert!(!preview["configuration_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value.as_str().unwrap().contains("backups")));
    let result = execute(&ctx, &snapshot.base, true, "mixed-backup-fixture").unwrap();
    assert_eq!(result["status"], "partial");
    assert_eq!(
        std::fs::read(dir.join("README.md")).unwrap(),
        b"PERSONAL_DOCUMENT"
    );
    assert_eq!(
        std::fs::read(dir.join(".claude.json.backup.1770000000000")).unwrap(),
        b"PRIVATE_BACKUP"
    );
    drop(snapshot);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn changed_or_hard_linked_backups_refuse_without_database_writes() {
    for relative in [
        ".claude/backups/.claude.json.backup.1770000000000",
        ".claude.json.backup.1770000000001",
    ] {
        let (path, ctx) = fixture();
        let file = ctx.home_dir.join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"PRIVATE_BACKUP").unwrap();
        let snapshot = Snapshot::capture(&ctx, true).unwrap();
        std::fs::write(&file, b"CHANGED_PRIVATE_BACKUP").unwrap();
        assert!(execute(&ctx, &snapshot.base, true, "stale-backup-fixture").is_err());
        assert!(!ctx.data_dir.exists());
        std::fs::hard_link(&file, ctx.home_dir.join("shared-alias.json")).unwrap();
        assert!(Snapshot::capture(&ctx, true).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"CHANGED_PRIVATE_BACKUP");
        assert!(!ctx.data_dir.exists());
        drop(snapshot);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn a_late_backup_alias_preserves_both_names() {
    for (relative, phase) in [
        (".claude/backups/.claude.json.backup.1770000000000", "files"),
        (".claude.json.backup.1770000000001", "configurations"),
    ] {
        let (path, ctx) = fixture();
        let file = ctx.home_dir.join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"PRIVATE_BACKUP").unwrap();
        let alias = ctx.home_dir.join("shared-alias.json");
        let snapshot = Snapshot::capture(&ctx, true).unwrap();
        let result = apply(
            &snapshot,
            &mut Progress::default(),
            || Ok(()),
            |current, _| {
                if current == phase {
                    std::fs::hard_link(&file, &alias)?;
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read(file).unwrap(), b"PRIVATE_BACKUP");
        assert_eq!(std::fs::read(alias).unwrap(), b"PRIVATE_BACKUP");
        drop(snapshot);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn late_project_or_unknown_content_protects_the_backup_directory() {
    for name in [".git", "README.md"] {
        let (path, ctx) = fixture();
        let dir = ctx.home_dir.join(".claude/backups");
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join(".claude.json.backup.1770000000000");
        std::fs::write(&file, b"PRIVATE_BACKUP").unwrap();
        let snapshot = Snapshot::capture(&ctx, true).unwrap();
        let result = apply(
            &snapshot,
            &mut Progress::default(),
            || Ok(()),
            |phase, _| {
                if phase == "files" {
                    std::fs::write(dir.join(name), b"PERSONAL_SOURCE")?;
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read(file).unwrap(), b"PRIVATE_BACKUP");
        assert_eq!(std::fs::read(dir.join(name)).unwrap(), b"PERSONAL_SOURCE");
        drop(snapshot);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn an_unpreviewed_home_backup_remains_and_prevents_complete_readback() {
    let (path, ctx) = fixture();
    let file = ctx.home_dir.join(".claude.json.backup.1770000000000");
    let snapshot = Snapshot::capture(&ctx, true).unwrap();
    apply(
        &snapshot,
        &mut Progress::default(),
        || Ok(()),
        |phase, _| {
            if phase == "files" {
                std::fs::write(&file, b"NEW_PRIVATE_BACKUP")?;
            }
            Ok(())
        },
    )
    .unwrap();
    let remaining = readback(&ctx, &snapshot).unwrap();
    assert!(remaining.issue.is_some());
    assert_eq!(std::fs::read(file).unwrap(), b"NEW_PRIVATE_BACKUP");
    drop(snapshot);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn unclassified_home_copies_are_visible_and_prevent_complete_results() {
    let (path, ctx) = fixture();
    let document = ctx.home_dir.join(".claude.json.backup.notes.md");
    std::fs::write(&document, b"PERSONAL_DOCUMENT").unwrap();
    let snapshot = Snapshot::capture(&ctx, true).unwrap();
    assert!(snapshot.has_unknown());
    assert!(snapshot.retained_data().iter().any(|item| item["path"]
        == json!(dunce::canonicalize(&document).unwrap())
        && item["reason"] == "unclassified"));
    let result = execute(&ctx, &snapshot.base, true, "unclassified-home-fixture").unwrap();
    assert_eq!(result["status"], "partial");
    assert_eq!(std::fs::read(document).unwrap(), b"PERSONAL_DOCUMENT");
    drop(snapshot);
    std::fs::remove_dir_all(path).unwrap();
}

// APFS rejects these bytes before the fixture can exercise inspection.
#[cfg(target_os = "linux")]
#[test]
fn undisplayable_copy_names_refuse_without_panicking_or_writing() {
    use std::os::unix::ffi::OsStringExt;
    for at_home in [true, false] {
        let (path, ctx) = fixture();
        let name = std::ffi::OsString::from_vec(if at_home {
            b".claude.json.backup.\xff".to_vec()
        } else {
            b"unknown-\xff".to_vec()
        });
        if at_home {
            std::fs::write(ctx.home_dir.join(name), b"PRIVATE_BACKUP").unwrap();
        } else {
            std::fs::create_dir(ctx.home_dir.join(".claude").join(name)).unwrap();
        }
        let result = std::panic::catch_unwind(|| {
            Snapshot::capture(&ctx, true).map(|snapshot| snapshot.data())
        });
        assert!(matches!(result, Ok(Err(_))));
        assert!(!ctx.data_dir.exists());
        assert!(ctx.home_dir.join(".claude/settings.json").exists());
        std::fs::remove_dir_all(path).unwrap();
    }
}
