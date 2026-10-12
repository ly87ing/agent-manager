//! 只读 system:discover 的分类、聚合、零写与参数反例。
//!
//! PATH 会影响 agents 候选解析（resolve_bin），需要改 PATH 的用例由 PATH_LOCK 串行，
//! 不改 PATH 的用例一律只查 `--only skills`，不受宿主已安装 CLI 影响。

use agent_manager_engine::{execute, store, Context, Request};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

static PATH_LOCK: Mutex<()> = Mutex::new(());

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    let base =
        std::env::temp_dir().join(format!("am-discover-test-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&base);
    let ctx = Context {
        data_dir: base.join("data"),
        home_dir: base.join("home"),
    };
    (base, ctx)
}

fn request(command: &str, args: &[&str]) -> Request {
    Request {
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        run_id: "discover-test".to_string(),
    }
}

struct PathGuard {
    original: Option<std::ffi::OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl PathGuard {
    /// 把 PATH 整体替换为只含 dir，宿主已安装的 CLI 不参与候选解析。
    fn only(dir: &Path) -> Self {
        let lock = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original = std::env::var_os("PATH");
        std::env::set_var("PATH", dir);
        PathGuard {
            original,
            _lock: lock,
        }
    }
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(orig) => std::env::set_var("PATH", orig),
            None => std::env::remove_var("PATH"),
        }
    }
}

fn link(source: &Path, link_path: &Path) {
    if source.is_dir() {
        agent_manager_engine::platform::link_dir(source, link_path).unwrap();
    } else {
        agent_manager_engine::platform::link_file(source, link_path).unwrap();
    }
}

fn make_skill(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("SKILL.md"), format!("# {name}\n")).unwrap();
    path
}

#[test]
fn discover_classifies_skill_entries_by_origin_and_stays_read_only() {
    let (root, ctx) = temp_ctx("classify");
    let claude_skills = ctx.home_dir.join(".claude/skills");
    std::fs::create_dir_all(&claude_skills).unwrap();

    // 1) 真实目录（助手官方路径内的原生安装）
    make_skill(&claude_skills, "synced");
    // 2) 外部路径链接（可软链接接管）
    let external_root = ctx.home_dir.join(".vendor-tool/skills");
    std::fs::create_dir_all(&external_root).unwrap();
    let alpha = make_skill(&external_root, "alpha");
    let beta = make_skill(&external_root, "beta");
    link(&alpha, &claude_skills.join("alpha"));
    link(&beta, &claude_skills.join("beta"));
    // 3) 断链
    link(
        &ctx.home_dir.join(".vendor-tool/skills/missing"),
        &claude_skills.join("ghost"),
    );
    // 4) 隐藏项（Codex .system 等内置目录）不进入候选
    std::fs::create_dir_all(claude_skills.join(".system")).unwrap();

    let env = execute(&ctx, &request("system:discover", &["--only", "skills"]));
    assert!(env.ok, "{:?}", env.errors);
    let items = env.data["skills"]["items"].as_array().unwrap();
    let find = |name: &str| {
        items
            .iter()
            .find(|item| item["name"] == name && item["agent"] == "claude-code")
            .unwrap_or_else(|| panic!("缺少条目 {name}"))
    };

    assert_eq!(find("synced")["origin"], "native-dir");
    assert_eq!(find("synced")["manage"], "native");
    assert_eq!(find("alpha")["origin"], "external-link");
    assert_eq!(find("alpha")["manage"], "symlink");
    assert_eq!(find("alpha")["suggested_collection"], "vendor-tool");
    assert_eq!(find("ghost")["origin"], "dead-link");
    assert_eq!(find("ghost")["manage"], "cleanup");
    assert!(
        !items.iter().any(|item| item["name"] == ".system"),
        "隐藏项不得进入候选"
    );

    // 路径一律以 ~ 形式输出，不泄漏绝对 home 路径
    assert_eq!(find("alpha")["link_path"], "~/.claude/skills/alpha");
    assert_eq!(find("alpha")["target"], "~/.vendor-tool/skills/alpha");

    // 按源根聚合出的建议集合定义可直接用于登记
    let groups = env.data["skills"]["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1, "只有一个外部源根");
    let group = &groups[0];
    assert_eq!(group["name"], "vendor-tool");
    assert_eq!(group["source_path"], "~/.vendor-tool/skills");
    assert_eq!(
        group["suggested_definition"]["source_path"],
        "~/.vendor-tool/skills"
    );
    assert_eq!(
        group["suggested_definition"]["skill_paths"],
        serde_json::json!(["alpha", "beta"])
    );
    assert_eq!(
        group["suggested_definition"]["agents"],
        serde_json::json!(["claude-code"])
    );

    let summary = &env.data["summary"];
    assert_eq!(summary["skills_symlink"], 2);
    assert_eq!(summary["skills_native"], 1);
    assert_eq!(summary["skills_cleanup"], 1);
    // 空库没有任何集合：内置推荐来源不是集合，不把同名链接判成计划外链接
    assert_eq!(summary["skills_collection"], 0);

    // 零写：不创建 data_dir 与数据库
    assert!(!ctx.data_dir.exists());
    assert!(!store::db_path(&ctx).exists());
    let _ = std::fs::remove_dir_all(root);
}

/// 计划外链接判定只认用户已保存的集合（原先依赖内置 agent-skills 固化进库）。
#[test]
fn discover_marks_collection_extra_link_for_user_collection() {
    let (root, ctx) = temp_ctx("extra-link");
    let claude_skills = ctx.home_dir.join(".claude/skills");
    std::fs::create_dir_all(&claude_skills).unwrap();
    let elsewhere = ctx.home_dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let coll_skill = make_skill(&elsewhere, "safe-merge-review");
    link(&coll_skill, &claude_skills.join("safe-merge-review"));

    let set = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.my-skills",
                "--value",
                r#"{"url":"https://example.invalid/my-skills.git","skill_paths":["safe-merge-review"]}"#,
            ],
        ),
    );
    assert!(set.ok, "{:?}", set.errors);
    let db = store::db_path(&ctx);
    let before = std::fs::read(&db).unwrap();

    let env = execute(&ctx, &request("system:discover", &["--only", "skills"]));
    assert!(env.ok, "{:?}", env.errors);
    let items = env.data["skills"]["items"].as_array().unwrap();
    let item = items
        .iter()
        .find(|item| item["name"] == "safe-merge-review")
        .expect("计划外链接必须列出");
    assert_eq!(item["origin"], "collection-extra-link");
    assert_eq!(item["manage"], "collection");
    assert_eq!(item["collection"], "my-skills");
    assert_eq!(env.data["summary"]["skills_collection"], 1);
    assert_eq!(std::fs::read(&db).unwrap(), before, "发现不得写库");
    let _ = std::fs::remove_dir_all(root);
}

/// skill_paths 是装技能的目录（`["skills"]`）时，按来源里实际发现的技能名判定计划外链接。
#[test]
fn discover_marks_collection_extra_link_for_directory_skill_paths() {
    let (root, ctx) = temp_ctx("extra-link-dir");
    let claude_skills = ctx.home_dir.join(".claude/skills");
    std::fs::create_dir_all(&claude_skills).unwrap();
    let elsewhere = ctx.home_dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    link(
        &make_skill(&elsewhere, "verification"),
        &claude_skills.join("verification"),
    );
    let mirror = ctx.data_dir.join("repositories").join("my-skills");
    make_skill(&mirror.join("skills"), "verification");

    let set = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.my-skills",
                "--value",
                r#"{"url":"https://example.invalid/my-skills.git","skill_paths":["skills"]}"#,
            ],
        ),
    );
    assert!(set.ok, "{:?}", set.errors);

    let env = execute(&ctx, &request("system:discover", &["--only", "skills"]));
    assert!(env.ok, "{:?}", env.errors);
    let item = env.data["skills"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "verification")
        .expect("计划外链接必须列出")
        .clone();
    assert_eq!(item["origin"], "collection-extra-link");
    assert_eq!(item["collection"], "my-skills");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn discover_skips_entries_already_managed_by_binding() {
    let (root, ctx) = temp_ctx("managed-skip");
    let claude_skills = ctx.home_dir.join(".claude/skills");
    std::fs::create_dir_all(&claude_skills).unwrap();
    let external_root = ctx.home_dir.join(".vendor-tool/skills");
    std::fs::create_dir_all(&external_root).unwrap();
    let alpha = make_skill(&external_root, "alpha");
    link(&alpha, &claude_skills.join("alpha"));

    // 写入一条 assigned binding（模拟已纳管）
    let conn = store::open_write(&ctx).unwrap();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS skill_bindings (
            collection TEXT NOT NULL, skill TEXT NOT NULL, agent TEXT NOT NULL,
            version TEXT NOT NULL, enabled INTEGER NOT NULL, assigned INTEGER NOT NULL,
            updated_at TEXT NOT NULL, PRIMARY KEY (collection, skill, agent));",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO skill_bindings VALUES ('vendor-tool','alpha','claude-code','v1',1,1,'now');",
        [],
    )
    .unwrap();
    drop(conn);

    let env = execute(&ctx, &request("system:discover", &["--only", "skills"]));
    assert!(env.ok, "{:?}", env.errors);
    let items = env.data["skills"]["items"].as_array().unwrap();
    assert!(
        !items.iter().any(|item| item["name"] == "alpha"),
        "已有 assigned binding 的条目不得再列为候选"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn discover_ignores_unsupported_agent() {
    let (root, ctx) = temp_ctx("agents");
    let bin_dir = ctx.home_dir.join(".local/bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let bin = bin_dir.join("qwen");
    std::fs::write(&bin, "#!/bin/sh\necho 'qwen 1.2.3'\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let _guard = PathGuard::only(&bin_dir);

    let env = execute(&ctx, &request("system:discover", &["--only", "agents"]));
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "DISCOVER_ERROR");
    assert!(!ctx.data_dir.exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn discover_ignores_unsupported_npm_agent() {
    let (root, ctx) = temp_ctx("agents-npm");
    let npm_root = ctx.home_dir.join(".npm-global");
    let pkg_bin = npm_root.join("lib/node_modules/@scope/gemini-cli/bin");
    std::fs::create_dir_all(&pkg_bin).unwrap();
    let real = pkg_bin.join("gemini.js");
    std::fs::write(&real, "#!/bin/sh\necho '1.0.0'\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let bin_dir = npm_root.join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    link(&real, &bin_dir.join("gemini"));
    let _guard = PathGuard::only(&bin_dir);

    let env = execute(&ctx, &request("system:discover", &["--only", "agents"]));
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "DISCOVER_ERROR");
    assert!(!ctx.data_dir.exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn discover_rejects_unknown_args_and_domains() {
    let (root, ctx) = temp_ctx("args");
    for args in [
        &["--only", "tools"][..],
        &["--dry-run"][..],
        &["--offline"][..],
        &["--only"][..],
    ] {
        let env = execute(&ctx, &request("system:discover", args));
        assert!(!env.ok, "参数 {:?} 必须被拒绝", args);
        assert_eq!(env.errors[0].code, "DISCOVER_ERROR");
    }
    assert!(!ctx.data_dir.exists(), "参数反例不得创建任何目录");
    let _ = std::fs::remove_dir_all(root);
}
