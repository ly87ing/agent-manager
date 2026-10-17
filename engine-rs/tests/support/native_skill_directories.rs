use super::*;

fn ordinary_tree(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    let mut tree = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for child in std::fs::read_dir(directory).unwrap() {
            let path = child.unwrap().path();
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            assert!(!metadata.file_type().is_symlink());
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                assert_eq!(
                    metadata.file_attributes()
                        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT,
                    0
                );
            }
            let key = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            if metadata.is_dir() {
                tree.insert(key, None);
                stack.push(path);
            } else {
                assert!(metadata.is_file());
                tree.insert(key, Some(std::fs::read(path).unwrap()));
            }
        }
    }
    tree
}

#[test]
fn sibling_directory_materialization_preserves_mixed_content_and_offline_forks() {
    for format in ["sha1", "sha256"] {
        let fixture = Fixture::new(format);
        std::fs::create_dir_all(fixture.source().join("assets/empty")).unwrap();
        std::fs::write(
            fixture.source().join("assets/payload"),
            b"directory bytes\n",
        )
        .unwrap();
        std::fs::write(fixture.source().join("node_modules"), b"assets").unwrap();
        fixture.git(&[
            "add",
            "--",
            "skills/alpha/assets",
            "skills/alpha/node_modules",
        ]);
        let id = fixture.git(&["hash-object", "-w", "--", "skills/alpha/node_modules"]);
        fixture.git(&[
            "update-index",
            "--cacheinfo",
            &format!("120000,{},skills/alpha/node_modules", id.trim()),
        ]);
        let metadata = git_metadata(&fixture.repo().join(".git"));
        fixture.action("add", "codex", true);
        let version = fixture.cell("codex")["version"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            version,
            "e2388ca42cc357ec6cd59312543ba8439ec277992216941198d5b34f95bdf115"
        );
        let snapshot = fixture.0.join("data/skill-versions").join(&version);
        let receipt = std::fs::read(snapshot.join("snapshot.json")).unwrap();
        let manifest: Value = serde_json::from_slice(&receipt).unwrap();
        assert_eq!(manifest["schema"], 4);
        assert_eq!(
            manifest["materialized_directory_links"][0]["path"],
            "node_modules"
        );
        assert_eq!(manifest["materialized_file_links"][0]["path"], "alias");
        let content = snapshot.join("content");
        let expected: BTreeMap<_, _> = [
            ("SKILL.md", Some(b"# Fixture\n".to_vec())),
            ("alias", Some(b"ordinary target bytes\n".to_vec())),
            ("z-target", Some(b"ordinary target bytes\n".to_vec())),
            ("assets", None),
            ("assets/empty", None),
            ("assets/payload", Some(b"directory bytes\n".to_vec())),
            ("node_modules", None),
            ("node_modules/empty", None),
            ("node_modules/payload", Some(b"directory bytes\n".to_vec())),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
        assert_eq!(ordinary_tree(&content), expected);
        fixture.action("add", "claude-code", true);
        assert_eq!(
            std::fs::read(snapshot.join("snapshot.json")).unwrap(),
            receipt
        );
        fixture.action("disable", "codex", true);
        let other = fixture.cell("claude-code");
        for name in ["assets/payload", "node_modules/payload"] {
            tamper_snapshot(&content.join(name), b"matching but changed\n");
        }
        fixture.action("enable", "codex", false);
        assert_eq!(fixture.cell("codex")["enabled"], false);
        assert_eq!(fixture.cell("claude-code"), other);
        for name in ["assets/payload", "node_modules/payload"] {
            tamper_snapshot(&content.join(name), b"directory bytes\n");
        }
        fixture.action("enable", "codex", true);
        assert_eq!(git_metadata(&fixture.repo().join(".git")), metadata);
        std::fs::rename(fixture.repo(), fixture.0.join("offline-source")).unwrap();
        let destination = fixture.0.join("personal");
        fixture.run(
            &[
                "skills:fork",
                "--only",
                "skill:upstream/alpha",
                "--dest",
                destination.to_str().unwrap(),
            ],
            true,
        );
        assert_eq!(ordinary_tree(&destination.join("alpha")), expected);
        std::fs::write(destination.join("alpha/node_modules/payload"), b"edited").unwrap();
        assert_eq!(ordinary_tree(&content), expected);
    }
}
