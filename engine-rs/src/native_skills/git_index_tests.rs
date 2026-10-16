use super::*;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

struct Fixture(PathBuf);

impl Fixture {
    fn new(format: &str) -> Self {
        let root = std::env::temp_dir().join(format!("am-git-index-{}", crate::store::uuid_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("empty-config"), b"").unwrap();
        std::fs::create_dir(root.join("templates")).unwrap();
        let fixture = Self(root);
        fixture.git(&[
            "init",
            "--quiet",
            &format!("--object-format={format}"),
            "--template=templates",
            ".",
        ]);
        std::fs::create_dir(fixture.0.join("skills")).unwrap();
        std::fs::write(fixture.0.join("skills/file"), b"target text").unwrap();
        fixture.git(&["add", "--", "skills/file"]);
        // A real index link with an ordinary worktree representation needs no link privilege.
        let object = fixture.git(&["hash-object", "-w", "skills/file"]);
        fixture.git(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("120000,{},skills/link", object.trim()),
        ]);
        fixture.git(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{},submodule", object.trim()),
        ]);
        fixture
    }

    fn git(&self, args: &[&str]) -> String {
        let mut command = crate::platform::git_command();
        command
            .current_dir(&self.0)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", self.0.join("empty-config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let result =
            crate::process::capture_group(&mut command, Duration::from_secs(20), 64 * 1024)
                .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("Retaining failed Git index fixture: {}", self.0.display());
            return;
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture_bytes(
    width: usize,
    version: u32,
    entries: &[(&[u8], u32, u16, Option<u16>)],
    extension: &[u8],
) -> Vec<u8> {
    let mut bytes = b"DIRC".to_vec();
    bytes.extend(version.to_be_bytes());
    bytes.extend((entries.len() as u32).to_be_bytes());
    for (path, mode, flags, extended) in entries {
        let start = bytes.len();
        let mut stat = [0; 40];
        stat[24..28].copy_from_slice(&mode.to_be_bytes());
        bytes.extend(stat);
        bytes.extend(vec![1; width]);
        bytes.extend((flags | path.len().min(0xfff) as u16).to_be_bytes());
        if let Some(extended) = extended {
            bytes.extend(extended.to_be_bytes());
        }
        bytes.extend(*path);
        bytes.resize(bytes.len() + 8 - (bytes.len() - start) % 8, 0);
    }
    bytes.extend(extension);
    seal(&mut bytes, width);
    bytes
}

fn seal(bytes: &mut Vec<u8>, width: usize) {
    let checksum = if width == 20 {
        sha1::Sha1::digest(&*bytes).to_vec()
    } else {
        Sha256::digest(&*bytes).to_vec()
    };
    bytes.extend(checksum);
}

#[test]
fn actual_git_v2_v3_sha1_sha256_and_skip_hash_remain_zero_write() {
    for format in ["sha1", "sha256"] {
        let fixture = Fixture::new(format);
        for version in ["2", "3"] {
            fixture.git(&["update-index", &format!("--index-version={version}")]);
            if version == "3" {
                fixture.git(&["update-index", "--skip-worktree", "skills/file"]);
            }
            let index_path = fixture.0.join(".git/index");
            let before = std::fs::read(&index_path).unwrap();
            let modified = std::fs::metadata(&index_path).unwrap().modified().unwrap();
            let index = read(&fixture.0).unwrap();
            assert_eq!(index.links, BTreeSet::from([b"skills/link".to_vec()]));
            assert_eq!(index.gitlinks, BTreeSet::from([b"submodule".to_vec()]));
            assert_eq!(std::fs::read(&index_path).unwrap(), before);
            assert_eq!(
                std::fs::metadata(&index_path).unwrap().modified().unwrap(),
                modified
            );
            let mut zero = before.clone();
            let width = if format == "sha1" { 20 } else { 32 };
            let end = zero.len() - width;
            zero[end..].fill(0);
            assert_eq!(decode(&zero).unwrap(), index);
            // Older Git can read this documented footer without supporting the writer option.
            std::fs::write(&index_path, &zero).unwrap();
            let modified = std::fs::metadata(&index_path).unwrap().modified().unwrap();
            assert_eq!(read(&fixture.0).unwrap(), index);
            assert_eq!(std::fs::read(&index_path).unwrap(), zero);
            assert_eq!(
                std::fs::metadata(&index_path).unwrap().modified().unwrap(),
                modified
            );
        }
        fixture.git(&["update-index", "--index-version=4"]);
        assert!(read(&fixture.0)
            .unwrap_err()
            .to_string()
            .contains("version"));
        fixture.git(&["update-index", "--index-version=2"]);
        fixture.git(&["update-index", "--split-index"]);
        assert!(read(&fixture.0).is_err());
        fixture.git(&["update-index", "--no-split-index"]);
        assert!(read(&fixture.0).is_ok());
        std::fs::write(fixture.0.join(".git/commondir"), b"../other").unwrap();
        assert!(read(&fixture.0).is_err());
    }
}

#[test]
fn optional_extensions_are_skipped_without_recursive_interpretation() {
    for width in [20, 32] {
        let entries = [(b"alias".as_slice(), 0o120000, 0, None)];
        for signature in [b"TREE", b"UNTR", b"ABCD"] {
            let mut extension = signature.to_vec();
            extension.extend(64u32.to_be_bytes());
            extension.extend([255; 64]);
            assert!(decode(&fixture_bytes(width, 2, &entries, &extension)).is_ok());
        }
        for signature in [b"link", b"sdir", b"abcd", b"1ABC", b"_ABC"] {
            let mut extension = signature.to_vec();
            extension.extend(0u32.to_be_bytes());
            assert!(decode(&fixture_bytes(width, 2, &entries, &extension)).is_err());
        }
        assert!(decode(&fixture_bytes(width, 2, &entries, b"TREE\xff\xff\xff\xff")).is_err());
    }
}

#[test]
fn malformed_entries_checksums_and_every_truncation_are_refused() {
    for width in [20, 32] {
        let valid = fixture_bytes(width, 2, &[(b"alias", 0o120000, 0, None)], b"");
        assert!(decode(&valid).is_ok());
        for end in 0..valid.len() {
            assert!(decode(&valid[..end]).is_err(), "accepted truncation {end}");
        }
        let mut changed = valid.clone();
        changed[20] ^= 1;
        assert!(decode(&changed).is_err());
        for path in [
            b"".as_slice(),
            b"/alias",
            b"../alias",
            b"a//b",
            b"a/./b",
            b"a/.git/b",
            b"alias/",
        ] {
            assert!(decode(&fixture_bytes(width, 2, &[(path, 0o120000, 0, None)], b"")).is_err());
        }
        for mode in [0o040000, 0o120644, 0o100600, 0xffff_ffff] {
            assert!(decode(&fixture_bytes(width, 2, &[(b"alias", mode, 0, None)], b"")).is_err());
        }
        for stage in [0x1000, 0x2000, 0x3000] {
            assert!(decode(&fixture_bytes(
                width,
                2,
                &[(b"alias", 0o120000, stage, None)],
                b""
            ))
            .is_err());
        }
        assert!(decode(&fixture_bytes(
            width,
            2,
            &[(b"alias", 0o120000, 0x4000, Some(0))],
            b""
        ))
        .is_err());
        assert!(decode(&fixture_bytes(
            width,
            3,
            &[(b"alias", 0o120000, 0x4000, Some(0x6000))],
            b""
        ))
        .is_ok());
        for flags in [1, 0x8000] {
            assert!(decode(&fixture_bytes(
                width,
                3,
                &[(b"alias", 0o120000, 0x4000, Some(flags))],
                b""
            ))
            .is_err());
        }
        for paths in [[b"a", b"a"], [b"b", b"a"]] {
            let entries = paths.map(|p| (p.as_slice(), 0o100644, 0, None));
            assert!(decode(&fixture_bytes(width, 2, &entries, b"")).is_err());
        }
        let entries = [
            (b"a".as_slice(), 0o100644, 0, None),
            (b"a-", 0o100644, 0, None),
            (b"a/b", 0o120000, 0, None),
        ];
        assert!(decode(&fixture_bytes(width, 2, &entries, b"")).is_err());
        let mut bad_padding = valid[..valid.len() - width].to_vec();
        *bad_padding.last_mut().unwrap() = 1;
        seal(&mut bad_padding, width);
        assert!(decode(&bad_padding).is_err());
    }
    assert!(decode(&vec![0; LIMIT + 1]).is_err());
}

#[test]
fn long_shared_directory_prefixes_do_not_require_per_component_searches() {
    let mut first = b"a/".repeat(900_000);
    first.push(b'b');
    let mut second = first.clone();
    *second.last_mut().unwrap() = b'c';
    let entries = [
        (first.as_slice(), 0o100644, 0, None),
        (second.as_slice(), 0o100644, 0, None),
    ];
    let bytes = fixture_bytes(20, 2, &entries, b"");
    assert!(bytes.len() < LIMIT);
    assert!(decode(&bytes).is_ok());
}
