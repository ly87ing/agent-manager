use super::*;
use serde_json::{json, Value};
use std::path::PathBuf;

const VERSION: &str = "e2388ca42cc357ec6cd59312543ba8439ec277992216941198d5b34f95bdf115";

#[test]
fn directory_namespace_refuses_case_collisions_and_unknown_non_ascii_case_mapping() {
    for paths in [["Assets", "assets"], ["É", "é"], ["K", "K"]] {
        let entries = paths
            .into_iter()
            .map(|path| (path.into(), Entry::Directory))
            .collect();
        assert!(portable_keys(&entries).is_err());
    }
    let entries = [
        ("中文".into(), Entry::Directory),
        ("中文/内容.txt".into(), Entry::File("digest".into())),
    ]
    .into_iter()
    .collect();
    portable_keys(&entries).unwrap();
}

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let root = Self(
            std::env::temp_dir().join(format!("am-directory-proof-{}", crate::store::uuid_v4())),
        );
        for path in ["assets/empty", "node_modules/empty"] {
            std::fs::create_dir_all(root.content().join(path)).unwrap();
        }
        for (path, bytes) in [
            ("SKILL.md", b"# Fixture\n".as_slice()),
            ("alias", b"ordinary target bytes\n"),
            ("z-target", b"ordinary target bytes\n"),
            ("assets/payload", b"directory bytes\n"),
            ("node_modules/payload", b"directory bytes\n"),
        ] {
            std::fs::write(root.content().join(path), bytes).unwrap();
        }
        root.write(&root.manifest());
        root
    }
    fn content(&self) -> PathBuf {
        self.0.join("skill-versions").join(VERSION).join("content")
    }
    fn manifest(&self) -> Value {
        json!({"schema":4,"version":VERSION,"materialized_file_links":[{"path":"alias","raw_target":"./z-target"}],"materialized_directory_links":[{"path":"node_modules","raw_target":"assets"}]})
    }
    fn write(&self, manifest: &Value) {
        std::fs::write(
            self.content().parent().unwrap().join("snapshot.json"),
            manifest.to_string(),
        )
        .unwrap();
    }
    fn verifies(&self) -> bool {
        super::super::snapshot::verify_skill_version(&self.0, VERSION).is_ok()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("Retained directory proof fixture: {}", self.0.display());
        } else {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[test]
fn directory_planning_rejects_nested_aliases_and_oversized_receipts_before_staging() {
    let mut entries: Entries = [
        ("assets".into(), Entry::Directory),
        ("assets/empty".into(), Entry::Directory),
        (
            "node_modules".into(),
            Entry::Link {
                raw_target: "assets".into(),
                target_relative: "assets".into(),
            },
        ),
        ("target-file".into(), Entry::File("digest".into())),
    ]
    .into_iter()
    .collect();
    assert!(
        plan(&entries).unwrap().is_some(),
        "Directory-only records must be supported"
    );
    entries.insert(
        "assets/alias".into(),
        Entry::Link {
            raw_target: "../target-file".into(),
            target_relative: "target-file".into(),
        },
    );
    assert!(plan(&entries).is_err());
    entries.remove("assets/alias");
    for i in 0..2000 {
        entries.insert(
            format!("file-alias-{i:04}"),
            Entry::Link {
                raw_target: "target-file".into(),
                target_relative: "target-file".into(),
            },
        );
    }
    let root = std::env::temp_dir().join(format!(
        "am-directory-receipt-limit-{}",
        crate::store::uuid_v4()
    ));
    let error = super::super::snapshot::snapshot_git_entries(
        &root.join("absent-source"),
        &root.join("absent-data"),
        &entries,
        true,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("manifest exceeds 64 KiB"),
        "{error}"
    );
    assert!(!root.exists());
}

#[test]
fn directory_receipts_require_exact_shapes_and_one_logical_namespace() {
    let root = Root::new();
    assert!(root.verifies());
    let original = root.manifest();
    let mut attacks = Vec::new();
    for field in ["materialized_directory_links", "materialized_file_links"] {
        let mut value = original.clone();
        value.as_object_mut().unwrap().remove(field);
        attacks.push(value);
        let mut value = original.clone();
        value[field] = Value::Null;
        attacks.push(value);
    }
    let mut value = original.clone();
    value["materialized_directory_links"] = json!([]);
    attacks.push(value);
    let mut value = original.clone();
    value["extra"] = json!("untrusted");
    attacks.push(value);
    let mut value = original.clone();
    value["materialized_directory_links"][0]["digest"] = json!("untrusted");
    attacks.push(value);
    let mut value = original.clone();
    value["materialized_file_links"][0]["path"] = json!("node_modules");
    attacks.push(value);
    let mut value = original.clone();
    value["materialized_file_links"][0]["path"] = json!("node_modules/payload");
    attacks.push(value);
    let mut value = original.clone();
    value["materialized_file_links"][0]["raw_target"] = json!("node_modules/payload");
    attacks.push(value);
    for raw in [
        "node_modules/../assets",
        "assets/empty",
        "missing/../assets",
        "ASSETS",
        "../assets",
        "assets/payload/../empty",
        "assets:stream",
        "C:assets",
        r"\\?\C:\assets",
    ] {
        let mut value = original.clone();
        value["materialized_directory_links"][0]["raw_target"] = json!(raw);
        attacks.push(value);
    }
    for value in attacks {
        root.write(&value);
        assert!(!root.verifies(), "{value}");
    }
    for text in [
        original
            .to_string()
            .replacen("\"schema\":4", "\"schema\":4,\"schema\":4", 1),
        original.to_string().replacen(
            "\"path\":\"node_modules\"",
            "\"path\":\"node_modules\",\"path\":\"node_modules\"",
            1,
        ),
    ] {
        std::fs::write(root.content().parent().unwrap().join("snapshot.json"), text).unwrap();
        assert!(!root.verifies());
    }
    root.write(&original);
    assert!(root.verifies());
}

#[test]
fn directory_fork_refuses_withdrawn_or_reinterpreted_receipts_before_copying() {
    let root = Root::new();
    let original = root.manifest();
    let destination = root.0.join("fork");
    let receipt = root.content().parent().unwrap().join("snapshot.json");
    for replacement in [
        None,
        Some(json!({"schema":2,"version":VERSION})),
        Some(
            json!({"schema":3,"version":VERSION,"materialized_file_links":original["materialized_file_links"]}),
        ),
    ] {
        root.write(&original);
        assert!(root.verifies());
        if let Some(replacement) = replacement {
            root.write(&replacement);
        } else {
            std::fs::remove_file(&receipt).unwrap();
        }
        assert!(super::super::snapshot_v2::copy_directory_snapshot(
            &root.content(),
            &destination,
            VERSION
        )
        .is_err());
        assert!(
            !destination.exists(),
            "Refused fallback must not copy or register a partial tree"
        );
    }
}

#[test]
fn directory_verification_reads_empty_directories_all_bytes_and_fixed_identity() {
    let root = Root::new();
    for path in ["node_modules/extra", "assets/extra", ".git"] {
        std::fs::create_dir(root.content().join(path)).unwrap();
        assert!(!root.verifies(), "{path}");
        std::fs::remove_dir(root.content().join(path)).unwrap();
    }
    std::fs::remove_dir(root.content().join("node_modules/empty")).unwrap();
    assert!(!root.verifies());
    std::fs::create_dir(root.content().join("node_modules/empty")).unwrap();
    for path in ["assets/payload", "node_modules/payload"] {
        std::fs::write(root.content().join(path), b"same changed bytes").unwrap();
    }
    assert!(!root.verifies());
    for path in ["assets/payload", "node_modules/payload"] {
        std::fs::write(root.content().join(path), b"directory bytes\n").unwrap();
    }
    assert!(root.verifies());
    let foreign = root.0.join("foreign");
    std::fs::create_dir(&foreign).unwrap();
    std::fs::write(foreign.join("marker"), b"unchanged").unwrap();
    crate::platform::link_dir(&foreign, &root.content().join("foreign-link")).unwrap();
    assert!(!root.verifies());
    assert_eq!(std::fs::read(foreign.join("marker")).unwrap(), b"unchanged");
    crate::platform::remove_link(root.content().join("foreign-link")).unwrap();
    let physical = super::super::snapshot_v2::bounded_physical_entries(&root.content()).unwrap();
    let physical_version = typed_hash(&physical);
    assert_ne!(physical_version, VERSION);
    let mut manifest = root.manifest();
    manifest["version"] = json!(physical_version);
    let forged = root.0.join("skill-versions").join(&physical_version);
    std::fs::rename(root.content().parent().unwrap(), &forged).unwrap();
    std::fs::write(forged.join("snapshot.json"), manifest.to_string()).unwrap();
    assert!(super::super::snapshot::verify_skill_version(&root.0, &physical_version).is_err());
}
