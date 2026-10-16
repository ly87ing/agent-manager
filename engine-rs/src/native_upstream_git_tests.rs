use super::*;
use crate::platform::Canon;

fn fixture_git(repo: &Path, root: &Path, args: &[&str]) -> String {
    let mut command = crate::platform::git_command();
    command
        .current_dir(repo)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", root.join("empty-config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid");
    let (code, stdout, stderr) = capture_exit(command, Duration::from_secs(10)).unwrap();
    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&stderr));
    String::from_utf8(stdout).unwrap().trim().into()
}

#[test]
fn explicit_git_probes_ignore_inherited_repository_selectors() {
    if let Some(root) = std::env::var_os("AM_GIT_PROBE_FIXTURE_CHILD") {
        let root = PathBuf::from(root);
        assert_eq!(
            root.parent().unwrap().canon().unwrap(),
            std::env::temp_dir().canon().unwrap()
        );
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("am-git-probe-"));
        assert_eq!(
            std::fs::read(root.join("fixture-owner")).unwrap(),
            b"owned fixture"
        );
        let ctx = Context {
            home_dir: root.join("home"),
            data_dir: root.join("data"),
        };
        let expected = std::fs::read_to_string(root.join("expected-head")).unwrap();
        let owned = root.join("owned");
        assert_eq!(
            git_local_head(&ctx, &owned).as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            git_ls_remote(&ctx, owned.to_str().unwrap(), "HEAD").as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(git_local_head(&ctx, &root.join("missing")), None);
        assert_eq!(git_ls_remote(&ctx, "--invalid", "HEAD"), None);
        return;
    }
    let root = std::env::temp_dir().join(format!("am-git-probe-{}", crate::store::uuid_v4()));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("fixture-owner"), b"owned fixture").unwrap();
    std::fs::write(root.join("empty-config"), b"").unwrap();
    std::fs::create_dir(root.join("templates")).unwrap();
    std::fs::create_dir(root.join("home")).unwrap();
    for name in ["owned", "other"] {
        let repo = root.join(name);
        std::fs::create_dir(&repo).unwrap();
        fixture_git(
            &repo,
            &root,
            &[
                "init",
                "--quiet",
                "--object-format=sha1",
                "--template=../templates",
            ],
        );
        std::fs::write(repo.join("item"), name).unwrap();
        fixture_git(&repo, &root, &["add", "--", "item"]);
        fixture_git(
            &repo,
            &root,
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ],
        );
    }
    let expected = fixture_git(&root.join("owned"), &root, &["rev-parse", "HEAD"]);
    let other_head = fixture_git(&root.join("other"), &root, &["rev-parse", "HEAD"]);
    assert_ne!(expected, other_head);
    std::fs::write(root.join("expected-head"), expected).unwrap();
    let other = root.join("other/.git");
    let before: Vec<_> = ["owned", "other"]
        .into_iter()
        .map(|name| {
            ["config", "index", "HEAD"]
                .into_iter()
                .map(|file| std::fs::read(root.join(name).join(".git").join(file)).unwrap())
                .collect::<Vec<_>>()
        })
        .collect();
    let mut child = crate::process::command(std::env::current_exe().unwrap());
    child
        .args(["--exact", "native_upstream::git_probe_tests::explicit_git_probes_ignore_inherited_repository_selectors", "--nocapture"])
        .env("AM_GIT_PROBE_FIXTURE_CHILD", &root)
        .env("GIT_CONFIG_GLOBAL", root.join("empty-config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env("GIT_DIR", &other)
        .env("GIT_COMMON_DIR", &other)
        .env("GIT_WORK_TREE", root.join("other"))
        .env("GIT_INDEX_FILE", other.join("index"))
        .env("GIT_OBJECT_DIRECTORY", other.join("objects"))
        .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", other.join("objects"))
        .env("GIT_NAMESPACE", "other-namespace");
    let (code, stdout, stderr) = capture_exit(child, Duration::from_secs(30)).unwrap();
    assert_eq!(
        code,
        0,
        "{}\n{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    for (name, before) in ["owned", "other"].into_iter().zip(before) {
        for (file, bytes) in ["config", "index", "HEAD"].into_iter().zip(before) {
            assert_eq!(
                std::fs::read(root.join(name).join(".git").join(file)).unwrap(),
                bytes
            );
        }
    }
    assert!(
        !root.join("data").exists(),
        "Read-only probes must not initialize app data"
    );
    std::fs::remove_dir_all(root).unwrap();
}
