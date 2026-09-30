// 部分辅助只给 Unix 专属的用例用
#![cfg_attr(not(unix), allow(dead_code))]
use agent_manager_engine::{execute, store, Context, Request};
use fs2::FileExt;
use std::fs::OpenOptions;
use std::path::PathBuf;

/// 建目录链接（Windows 上没有符号链接权限时是联接），与引擎用的同一个函数。
fn link_dir(
    target: impl AsRef<std::path::Path>,
    link: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    agent_manager_engine::platform::link_dir(target.as_ref(), link.as_ref())
}

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    let base = std::env::temp_dir().join(format!("am-native-test-{}-{}", std::process::id(), tag));
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
        run_id: "test-run-1".to_string(),
    }
}

fn request_with_run(command: &str, args: &[&str], run_id: &str) -> Request {
    Request {
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        run_id: run_id.to_string(),
    }
}
/// 空库：内置技能仓库只是推荐来源（recommendations），不是元素；status 不建库、不写文件。
#[test]
fn skills_status_without_db_offers_recommendation_not_element() {
    let (root, context) = temp_ctx("skills-builtin");
    let env = execute(&context, &request("skills:status", &[]));
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["domain"], "skills");
    assert_eq!(
        env.data["elements"],
        serde_json::json!([]),
        "空库不得有任何集合元素（内置仓库不再是元素）"
    );
    let recs = env.data["recommendations"].as_array().unwrap();
    let rec = recs
        .iter()
        .find(|r| r["name"] == "agent-skills")
        .expect("内置仓库以推荐来源出现");
    assert_eq!(rec["url"], "https://github.com/ly87ing/agent-skills.git");
    assert_eq!(rec["added"], false);
    assert_eq!(rec["definition"]["url"], rec["url"]);
    assert_eq!(rec["definition"]["skill_paths"], rec["skill_paths"]);
    assert!(
        rec["definition"].get("agents").is_none(),
        "添加推荐不限定也不分配助手"
    );
    assert_eq!(env.data["installed"], serde_json::json!([]));
    // 零写库保证
    assert!(!context.data_dir.exists());
    assert!(!store::db_path(&context).exists());
    assert!(!context.home_dir.exists(), "status 不得创建助手目录");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn skills_collection_and_skill_lifecycle_and_cascading() {
    let (root, context) = temp_ctx("skills-lifecycle");

    // 准备一个本地受管仓库 fixture：包含两个 skill（alpha 与 beta）
    let repo_dir = context.data_dir.join("repositories").join("test-skills");
    std::fs::create_dir_all(repo_dir.join("alpha")).unwrap();
    std::fs::create_dir_all(repo_dir.join("beta")).unwrap();
    std::fs::write(repo_dir.join("alpha").join("SKILL.md"), "# Alpha skill\nv1").unwrap();
    std::fs::write(repo_dir.join("beta").join("SKILL.md"), "# Beta skill\nv1").unwrap();
    std::fs::write(
        repo_dir.join(".agent-manager-owned"),
        "{\"managed_by\":\"agent-manager\"}\n",
    )
    .unwrap();

    // 写入集合定义到 policy
    let set_res = execute(
        &context,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.test-skills",
                "--value",
                "{\"url\":\"https://example.com/test-skills.git\",\"skill_paths\":[\"alpha\",\"beta\"],\"agents\":[\"codex\",\"claude-code\"]}",
            ],
        ),
    );
    assert!(set_res.ok, "{:?}", set_res.errors);

    // 1. status 查看新发现技能（进 discovery_inbox）
    let status = execute(&context, &request("skills:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    let elements = status.data["elements"].as_array().unwrap();
    let coll_el = elements
        .iter()
        .find(|el| el["id"] == "skill:test-skills")
        .unwrap();
    assert_eq!(coll_el["state"], "managed-ok");
    assert_eq!(coll_el["observed"]["repo_cached"], true);
    let inbox = coll_el["observed"]["discovery_inbox"].as_array().unwrap();
    assert!(inbox.iter().any(|s| s == "alpha"));
    assert!(inbox.iter().any(|s| s == "beta"));

    // 2. 应用前 status 回读：alpha 在 codex 下未分配且目标入口不存在（零写保证）
    let codex_link = context.home_dir.join(".codex").join("skills").join("alpha");
    let pre = execute(&context, &request("skills:status", &[]));
    assert!(pre.ok, "{:?}", pre.errors);
    let pre_alpha = pre.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|el| el["id"] == "skill:test-skills/alpha")
        .unwrap();
    assert_eq!(pre_alpha["observed"]["agents"]["codex"]["assigned"], false);
    assert_eq!(
        pre_alpha["observed"]["agents"]["codex"]["state"],
        "not-assigned"
    );
    assert!(!codex_link.exists(), "status 回读不应写文件");

    // 3. --dry-run 已退役：严格拒绝、不取锁、目标与 DB 零变化
    let dry = execute(
        &context,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:test-skills/alpha",
                "--action",
                "add",
                "--agent",
                "codex",
                "--dry-run",
            ],
        ),
    );
    assert!(!dry.ok, "--dry-run 必须被严格拒绝");
    assert_eq!(dry.errors[0].code, "SKILL_ERROR");
    assert!(dry.errors[0].message.contains("--dry-run"));
    assert!(!dry.lock.held, "--dry-run 拒绝必须先于取锁");
    assert!(!codex_link.exists(), "--dry-run 拒绝不得写软连接");
    if let Some(conn) = store::open_readonly(&context).unwrap() {
        let count: i64 = conn
            .query_row("SELECT count(*) FROM skill_bindings", [], |r| r.get(0))
            .unwrap_or(0);
        assert_eq!(count, 0, "--dry-run 拒绝不得写入绑定");
    }

    // 4. real apply: add alpha @ codex
    let apply = execute(
        &context,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:test-skills/alpha",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(apply.ok, "{:?}", apply.errors);
    assert!(codex_link.exists(), "必须已创建软连接");
    let meta = std::fs::symlink_metadata(&codex_link).unwrap();
    assert!(meta.file_type().is_symlink(), "目标必须是符号链接");
    let target = std::fs::read_link(&codex_link).unwrap();
    assert!(
        target.to_string_lossy().contains("skill-versions"),
        "必须指向不可变快照目录"
    );

    // 5. status 验证已链接状态
    let status = execute(&context, &request("skills:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    let alpha_el = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|el| el["id"] == "skill:test-skills/alpha")
        .unwrap();
    assert_eq!(alpha_el["observed"]["agents"]["codex"]["state"], "linked");
    assert_eq!(alpha_el["observed"]["agents"]["codex"]["assigned"], true);
    assert_eq!(alpha_el["observed"]["agents"]["codex"]["enabled"], true);

    // 6. disable alpha @ codex: 软连接被撤下，分配保留
    let dis = execute(
        &context,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:test-skills/alpha",
                "--action",
                "disable",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(dis.ok, "{:?}", dis.errors);
    assert!(!codex_link.exists(), "停用后软连接必须不存在");

    // 7. re-enable alpha @ codex: 软连接按 DB 记录恢复
    let en = execute(
        &context,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:test-skills/alpha",
                "--action",
                "enable",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(en.ok, "{:?}", en.errors);
    assert!(codex_link.exists(), "重新启用后软连接必须恢复");

    // 8. 集合级停用 (cascade disable): 级联为链接不存在
    let col_dis = execute(
        &context,
        &request(
            "skills:apply",
            &["--only", "skill:test-skills", "--action", "disable"],
        ),
    );
    assert!(col_dis.ok, "{:?}", col_dis.errors);
    assert!(!codex_link.exists(), "集合停用后软连接必须不存在");

    // 9. 集合级恢复 (cascade enable): 恢复原已启用项
    let col_en = execute(
        &context,
        &request(
            "skills:apply",
            &["--only", "skill:test-skills", "--action", "enable"],
        ),
    );
    assert!(col_en.ok, "{:?}", col_en.errors);
    assert!(codex_link.exists(), "集合重新启用后原项必须恢复软连接");

    // 10. 删除集合: 清除软连接并写入 tombstone，源目录保留
    let col_rm = execute(
        &context,
        &request(
            "skills:apply",
            &["--only", "skill:test-skills", "--action", "remove"],
        ),
    );
    assert!(col_rm.ok, "{:?}", col_rm.errors);
    assert!(!codex_link.exists(), "集合删除后软连接必须清理");
    assert!(repo_dir.exists(), "源目录必须保留在本地");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn skills_safety_unmanaged_content_rejected() {
    let (root, context) = temp_ctx("skills-safety");
    let repo_dir = context.data_dir.join("repositories").join("test-safe");
    std::fs::create_dir_all(repo_dir.join("alpha")).unwrap();
    std::fs::write(repo_dir.join("alpha").join("SKILL.md"), "# content").unwrap();

    let set_res = execute(
        &context,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.test-safe",
                "--value",
                "{\"url\":\"https://example.com/test-safe.git\",\"skill_paths\":[\"alpha\"],\"agents\":[\"codex\"]}",
            ],
        ),
    );
    assert!(set_res.ok);

    // 在目标入口放一个真实的普通目录（非受管内容）
    let target_entry = context.home_dir.join(".codex").join("skills").join("alpha");
    std::fs::create_dir_all(&target_entry).unwrap();
    std::fs::write(target_entry.join("my-user-data.txt"), "important data").unwrap();

    // 尝试 add 时必须拒绝，绝不能静默覆盖或删除用户的目录
    let add_res = execute(
        &context,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:test-safe/alpha",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(!add_res.ok, "必须拒绝覆盖外部内容");
    assert_eq!(add_res.errors[0].code, "SKILL_ERROR");
    assert!(add_res.errors[0].message.contains("外部非受管内容"));
    assert!(
        target_entry.join("my-user-data.txt").exists(),
        "用户数据绝不损坏"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn skills_apply_rejects_contended_lock_and_audits_operations() {
    let (base, ctx) = temp_ctx("skills-lock-audit");
    let repo_dir = ctx.data_dir.join("repositories").join("test-repo");
    std::fs::create_dir_all(repo_dir.join("demo-skill")).unwrap();
    std::fs::write(repo_dir.join("demo-skill").join("SKILL.md"), "# demo skill").unwrap();

    let set_res = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.test-repo",
                "--value",
                "{\"url\":\"https://example.com/test-repo.git\",\"skill_paths\":[\"demo-skill\"],\"agents\":[\"codex\"]}",
            ],
        ),
    );
    assert!(set_res.ok);

    // 1. 模拟并发持有跨进程 apply 锁
    std::fs::create_dir_all(&ctx.data_dir).unwrap();
    let lock_path = ctx.data_dir.join("agent-manager.apply.lock");
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    lock_file.lock_exclusive().unwrap();

    let worker_ctx = Context {
        data_dir: ctx.data_dir.clone(),
        home_dir: ctx.home_dir.clone(),
    };
    let worker = std::thread::spawn(move || {
        execute(
            &worker_ctx,
            &request_with_run(
                "skills:apply",
                &[
                    "--only",
                    "skill:test-repo/demo-skill",
                    "--action",
                    "add",
                    "--agent",
                    "codex",
                ],
                "skills-contended-run",
            ),
        )
    });
    let contended_res = worker.join().unwrap();
    assert!(!contended_res.ok, "锁竞争时必须失败");
    assert_eq!(contended_res.errors[0].code, "SKILL_ERROR");
    assert!(!contended_res.lock.held);
    assert!(!ctx
        .home_dir
        .join(".codex")
        .join("skills")
        .join("demo-skill")
        .exists());

    // 锁竞争失败不会执行或记录 run
    let runs = execute(
        &ctx,
        &request_with_run(
            "runs:get",
            &["--run", "skills-contended-run"],
            "skills-contended-run",
        ),
    );
    assert!(!runs.ok);
    assert_eq!(runs.errors[0].code, "RUN_NOT_FOUND");

    // 2. 释放锁，执行成功的 apply
    drop(lock_file);

    let success_res = execute(
        &ctx,
        &request_with_run(
            "skills:apply",
            &[
                "--only",
                "skill:test-repo/demo-skill",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
            "skills-success-run",
        ),
    );
    assert!(success_res.ok, "{:?}", success_res.errors);
    assert!(ctx
        .home_dir
        .join(".codex")
        .join("skills")
        .join("demo-skill")
        .exists());

    // 验证成功记录入 operation_logs
    let runs_success = execute(
        &ctx,
        &request_with_run("runs:get", &["--run", "skills-success-run"], "audit-check"),
    );
    assert!(runs_success.ok, "{:?}", runs_success.errors);
    let ops = runs_success.data["operations"].as_array().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["operation_type"], "skills:add");
    assert_eq!(ops[0]["target"], "test-repo/demo-skill@codex");
    assert!(ops[0]["detail"].get("version").is_some());

    // 3. 删除 repo 中的 skill 目录制造真实执行阶段失败，验证失败记录落入 operation_logs 且不含敏感信息
    std::fs::remove_dir_all(repo_dir.join("demo-skill")).unwrap();
    let fail_res = execute(
        &ctx,
        &request_with_run(
            "skills:apply",
            &[
                "--only",
                "skill:test-repo/demo-skill",
                "--action",
                "update",
                "--agent",
                "codex",
            ],
            "skills-fail-run",
        ),
    );
    assert!(!fail_res.ok);
    assert_eq!(fail_res.errors[0].code, "SKILL_ERROR");

    let runs_fail = execute(
        &ctx,
        &request_with_run("runs:get", &["--run", "skills-fail-run"], "audit-check-2"),
    );
    assert!(runs_fail.ok, "{:?}", runs_fail.errors);
    let fail_ops = runs_fail.data["operations"].as_array().unwrap();
    assert_eq!(fail_ops.len(), 1);
    assert_eq!(fail_ops[0]["operation_type"], "skills:update");
    assert_eq!(fail_ops[0]["detail"]["ok"], false);

    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn skills_disable_collection_does_not_affect_other_collections() {
    let (base, ctx) = temp_ctx("skills-isolate");
    let repo_a = ctx.data_dir.join("repositories").join("repo-a");
    let repo_b = ctx.data_dir.join("repositories").join("repo-b");
    std::fs::create_dir_all(repo_a.join("alpha")).unwrap();
    std::fs::write(repo_a.join("alpha").join("SKILL.md"), "# alpha").unwrap();
    std::fs::create_dir_all(repo_b.join("beta")).unwrap();
    std::fs::write(repo_b.join("beta").join("SKILL.md"), "# beta").unwrap();

    let set_a = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.repo-a",
                "--value",
                "{\"url\":\"https://example.com/repo-a.git\",\"skill_paths\":[\"alpha\"],\"agents\":[\"codex\"]}",
            ],
        ),
    );
    assert!(set_a.ok);
    let set_b = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.repo-b",
                "--value",
                "{\"url\":\"https://example.com/repo-b.git\",\"skill_paths\":[\"beta\"],\"agents\":[\"codex\"]}",
            ],
        ),
    );
    assert!(set_b.ok);

    // 分别为两个集合添加技能到 codex
    let add_a = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:repo-a/alpha",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(add_a.ok);
    let add_b = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:repo-b/beta",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(add_b.ok);

    let codex_skills = ctx.home_dir.join(".codex").join("skills");
    assert!(codex_skills.join("alpha").exists());
    assert!(codex_skills.join("beta").exists());

    // 1. 停用前 status 回读：repo-a/alpha 与 repo-b/beta 各自 linked，分属不同集合
    let pre = execute(&ctx, &request("skills:status", &[]));
    assert!(pre.ok, "{:?}", pre.errors);
    let pre_elements = pre.data["elements"].as_array().unwrap();
    let alpha_el = pre_elements
        .iter()
        .find(|el| el["id"] == "skill:repo-a/alpha")
        .unwrap();
    assert_eq!(alpha_el["observed"]["agents"]["codex"]["state"], "linked");
    let beta_el = pre_elements
        .iter()
        .find(|el| el["id"] == "skill:repo-b/beta")
        .unwrap();
    assert_eq!(beta_el["observed"]["agents"]["codex"]["state"], "linked");

    // 2. apply 集合 repo-a disable：alpha 被撤下，beta 依然保留
    let dis_a = execute(
        &ctx,
        &request(
            "skills:apply",
            &["--only", "skill:repo-a", "--action", "disable"],
        ),
    );
    assert!(dis_a.ok);
    assert!(
        !codex_skills.join("alpha").exists(),
        "alpha 软链接必须被撤下"
    );
    assert!(
        codex_skills.join("beta").exists(),
        "beta 软链接绝不能被触碰"
    );

    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn skills_collection_external_conflict_aborts_with_zero_changes() {
    let (base, ctx) = temp_ctx("skills-conflict-abort");
    let repo_dir = ctx.data_dir.join("repositories").join("multi-skills");
    std::fs::create_dir_all(repo_dir.join("skill-1")).unwrap();
    std::fs::write(repo_dir.join("skill-1").join("SKILL.md"), "# 1").unwrap();
    std::fs::create_dir_all(repo_dir.join("skill-2")).unwrap();
    std::fs::write(repo_dir.join("skill-2").join("SKILL.md"), "# 2").unwrap();

    let set = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.multi-skills",
                "--value",
                "{\"url\":\"https://example.com/multi.git\",\"skill_paths\":[\"skill-1\",\"skill-2\"],\"agents\":[\"codex\"]}",
            ],
        ),
    );
    assert!(set.ok);

    let add_1 = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:multi-skills/skill-1",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(add_1.ok);
    let add_2 = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:multi-skills/skill-2",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(add_2.ok);

    let codex_skills = ctx.home_dir.join(".codex").join("skills");
    let link_1 = codex_skills.join("skill-1");
    let link_2 = codex_skills.join("skill-2");
    assert!(link_1.exists());
    assert!(link_2.exists());

    // 人工替换 skill-2 目标为普通目录及用户文件（模拟外部非受管冲突）
    agent_manager_engine::platform::remove_link(&link_2).unwrap();
    std::fs::create_dir_all(&link_2).unwrap();
    std::fs::write(link_2.join("user-file.txt"), "important").unwrap();

    // 停用集合时遭遇冲突：必须 fail-closed 报错并中止
    let dis_res = execute(
        &ctx,
        &request(
            "skills:apply",
            &["--only", "skill:multi-skills", "--action", "disable"],
        ),
    );
    assert!(!dis_res.ok, "遇到非受管冲突必须报错拒绝");
    assert_eq!(dis_res.errors[0].code, "SKILL_ERROR");

    // 零变更保证：skill-1 依然存在且未被撤除，skill-2 的用户文件未损坏
    assert!(link_1.exists(), "skill-1 必须未受影响保留");
    assert!(link_2.join("user-file.txt").exists(), "用户数据绝不丢失");

    // 数据库开关保证：multi-skills.enabled 必须依然为 true（未被写入 false）
    let status = execute(&ctx, &request("skills:status", &[]));
    assert!(status.ok);
    let elements = status.data["elements"].as_array().unwrap();
    let coll = elements
        .iter()
        .find(|e| e["id"] == "skill:multi-skills")
        .unwrap();
    assert_eq!(coll["policy"]["enabled"], true, "数据库全局开关绝不能变更");

    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn skills_enable_rejected_when_collection_or_agent_disabled() {
    let (base, ctx) = temp_ctx("skills-cascade-reject");
    let repo_dir = ctx.data_dir.join("repositories").join("coll-gate");
    std::fs::create_dir_all(repo_dir.join("skill-gate")).unwrap();
    std::fs::write(repo_dir.join("skill-gate").join("SKILL.md"), "# gate").unwrap();

    let set = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.coll-gate",
                "--value",
                "{\"url\":\"https://example.com/gate.git\",\"skill_paths\":[\"skill-gate\"],\"agents\":[\"codex\"]}",
            ],
        ),
    );
    assert!(set.ok);

    // 1. 添加技能，并单项停用
    let add_res = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:coll-gate/skill-gate",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(add_res.ok);
    let dis_res = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:coll-gate/skill-gate",
                "--action",
                "disable",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(dis_res.ok);

    // 2. 集合停用时：尝试启用单技能必须被拒绝
    let dis_coll = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.coll-gate.enabled",
                "--value",
                "false",
            ],
        ),
    );
    assert!(dis_coll.ok);

    // 集合停用时：preflight 在取锁前拒绝 apply，且不写任何目标
    let apply_fail = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:coll-gate/skill-gate",
                "--action",
                "enable",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(!apply_fail.ok, "集合停用时 apply 必须拒绝");
    assert_eq!(apply_fail.errors[0].code, "SKILL_ERROR");
    assert!(!apply_fail.lock.held, "preflight 拒绝必须先于取锁");
    assert!(
        !ctx.home_dir
            .join(".codex")
            .join("skills")
            .join("skill-gate")
            .exists(),
        "被拒绝的 enable 不得建立软连接"
    );

    // 3. 恢复集合，但停用 Agent：尝试启用单技能仍必须被拒绝
    let en_coll = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.coll-gate.enabled",
                "--value",
                "true",
            ],
        ),
    );
    assert!(en_coll.ok);

    let dis_agent = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "agents.codex.enabled",
                "--value",
                "false",
            ],
        ),
    );
    assert!(dis_agent.ok);

    // Agent 停用时：preflight 同样在取锁前拒绝 apply，且不写任何目标
    let apply_fail_2 = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:coll-gate/skill-gate",
                "--action",
                "enable",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(!apply_fail_2.ok, "Agent 停用时 apply 必须拒绝");
    assert_eq!(apply_fail_2.errors[0].code, "SKILL_ERROR");
    assert!(!apply_fail_2.lock.held, "preflight 拒绝必须先于取锁");
    assert!(
        !ctx.home_dir
            .join(".codex")
            .join("skills")
            .join("skill-gate")
            .exists(),
        "被拒绝的 enable 不得建立软连接"
    );

    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn skills_transaction_failure_rolls_back_symlinks() {
    let (base, ctx) = temp_ctx("skills-tx-rollback");
    let repo_dir = ctx.data_dir.join("repositories").join("rollback-coll");
    std::fs::create_dir_all(repo_dir.join("rb-skill")).unwrap();
    std::fs::write(repo_dir.join("rb-skill").join("SKILL.md"), "# rb").unwrap();

    let set = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.rollback-coll",
                "--value",
                "{\"url\":\"https://example.com/rb.git\",\"skill_paths\":[\"rb-skill\"],\"agents\":[\"codex\"]}",
            ],
        ),
    );
    assert!(set.ok);

    // 通过 SQLite trigger 在 operation_logs 插入 skills:add 时注入真实数据库失败
    {
        let db_path = ctx.data_dir.join("native-manager.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TRIGGER test_fail_skills_add
             BEFORE INSERT ON operation_logs
             WHEN NEW.operation_type = 'skills:add'
             BEGIN
                 SELECT RAISE(ABORT, 'injected db failure during skills:add');
             END;",
            [],
        )
        .unwrap();
    }

    let fail_res = execute(
        &ctx,
        &request_with_run(
            "skills:apply",
            &[
                "--only",
                "skill:rollback-coll/rb-skill",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
            "test-rollback-trigger",
        ),
    );
    assert!(!fail_res.ok);
    assert_eq!(fail_res.errors[0].code, "SKILL_ERROR");

    // 验证回滚生效：软链接被回滚清理，未留下孤立链接
    let link = ctx.home_dir.join(".codex").join("skills").join("rb-skill");
    assert!(!link.exists(), "事务失败时已建软链接必须完全回滚撤销");

    // 数据库绑定验证：通过公开 store 只读查询 SQLite 与 status 视图，证明未提交任何有效分配记录
    if let Some(conn) = store::open_readonly(&ctx).unwrap() {
        let count: i64 = conn
            .query_row("SELECT count(*) FROM skill_bindings", [], |r| r.get(0))
            .unwrap_or(0);
        assert_eq!(count, 0, "事务失败时数据库绑定必须回滚为空");
    }
    let status_env = execute(&ctx, &request("skills:status", &[]));
    assert!(status_env.ok, "{:?}", status_env.errors);
    let elements = status_env.data["elements"].as_array().unwrap();
    let skill_cell = elements
        .iter()
        .find(|el| el["id"] == "skill:test-skills/rb-skill");
    assert!(
        skill_cell.is_none()
            || skill_cell.unwrap()["observed"]["agents"]["codex"]["assigned"] == false
    );

    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn skills_snapshot_rejects_external_symlinks() {
    let (base, ctx) = temp_ctx("skills-escaping-symlink");
    let repo_dir = ctx.data_dir.join("repositories").join("symlink-test");
    let skill_dir = repo_dir.join("escape-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), "# escape test").unwrap();

    // 1. 绝对路径软链接：必须被拒绝
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/hosts", skill_dir.join("abs-link")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(
        "C:\\Windows\\System32\\drivers\\etc\\hosts",
        skill_dir.join("abs-link"),
    )
    .unwrap();

    let hash_err = agent_manager_engine::native_skills::hash_skill_dir(&skill_dir);
    assert!(hash_err.is_err(), "绝对路径软连接必须在哈希与快照前被拒绝");
    assert!(hash_err.unwrap_err().to_string().contains("绝对路径"));

    // 2. 逃逸出技能根目录的相对软链接：必须被拒绝
    agent_manager_engine::platform::remove_link(skill_dir.join("abs-link")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("../../outside-secret.txt", skill_dir.join("rel-escape")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file("..\\..\\outside-secret.txt", skill_dir.join("rel-escape"))
        .unwrap();

    let escape_err = agent_manager_engine::native_skills::hash_skill_dir(&skill_dir);
    assert!(escape_err.is_err(), "跨根目录逃逸相对软链接必须被拒绝");
    assert!(escape_err
        .unwrap_err()
        .to_string()
        .contains("逃逸出技能根目录"));

    // 3. 安全的技能内部相对软链接：必须被允许
    agent_manager_engine::platform::remove_link(skill_dir.join("rel-escape")).unwrap();
    std::fs::create_dir_all(skill_dir.join("subdir")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        "../SKILL.md",
        skill_dir.join("subdir").join("internal-link"),
    )
    .unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(
        "..\\SKILL.md",
        skill_dir.join("subdir").join("internal-link"),
    )
    .unwrap();

    let valid_hash = agent_manager_engine::native_skills::hash_skill_dir(&skill_dir);
    assert!(
        valid_hash.is_ok(),
        "合法的技能内部相对软链接必须支持快照: {:?}",
        valid_hash.err()
    );

    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn skills_apply_rejects_dry_run_without_lock_or_writes() {
    let (base, ctx) = temp_ctx("skills-dry-run-reject");
    let repo_dir = ctx.data_dir.join("repositories").join("dry-coll");
    let skill_dir = repo_dir.join("my-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), "# dry run test").unwrap();

    // 先写入策略，确保 native-manager.db 真实存在
    let set = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.dry-coll",
                "--value",
                "{\"url\":\"https://example.com/dry.git\",\"skill_paths\":[\"my-skill\"],\"agents\":[\"codex\"]}",
            ],
        ),
    );
    assert!(set.ok, "{:?}", set.errors);
    assert!(
        ctx.data_dir.join("native-manager.db").exists(),
        "测试前数据库必须已存在"
    );

    let lock_path = ctx.data_dir.join("agent-manager.apply.lock");
    assert!(!lock_path.exists(), "执行 dry-run 前不能有锁文件");
    let skill_link = ctx.home_dir.join(".codex").join("skills").join("my-skill");
    let count_ops = |ctx: &Context| -> i64 {
        store::open_readonly(ctx)
            .unwrap()
            .map(|conn| {
                conn.query_row("SELECT count(*) FROM operation_logs", [], |r| r.get(0))
                    .unwrap_or(0)
            })
            .unwrap_or(0)
    };
    let ops_baseline = count_ops(&ctx);

    // 1. 单 Skill --dry-run：严格拒绝、不取锁、目标与 DB 零变化
    let dry_skill = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:dry-coll/my-skill",
                "--action",
                "add",
                "--agent",
                "codex",
                "--dry-run",
            ],
        ),
    );
    assert!(!dry_skill.ok, "--dry-run 必须被严格拒绝");
    assert_eq!(dry_skill.errors[0].code, "SKILL_ERROR");
    assert!(dry_skill.errors[0].message.contains("--dry-run"));
    assert!(!dry_skill.lock.held, "--dry-run 拒绝必须先于取锁");
    assert!(
        !lock_path.exists(),
        "DB 已存在时 skills:apply --dry-run 拒绝不得创建锁文件"
    );
    assert!(!skill_link.exists(), "--dry-run 拒绝不得写软连接");

    // 2. 集合级 --dry-run：同样拒绝且零写
    let dry_coll = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:dry-coll",
                "--action",
                "enable",
                "--dry-run",
            ],
        ),
    );
    assert!(!dry_coll.ok, "集合级 --dry-run 必须被严格拒绝");
    assert_eq!(dry_coll.errors[0].code, "SKILL_ERROR");
    assert!(!dry_coll.lock.held, "集合级 --dry-run 拒绝必须先于取锁");
    assert!(
        !lock_path.exists(),
        "集合级 skills:apply --dry-run 拒绝不得创建锁文件"
    );

    // 3. DB 不变：无绑定写入、操作审计记录数与拒绝前一致
    if let Some(conn) = store::open_readonly(&ctx).unwrap() {
        let bindings: i64 = conn
            .query_row("SELECT count(*) FROM skill_bindings", [], |r| r.get(0))
            .unwrap_or(0);
        assert_eq!(bindings, 0, "--dry-run 拒绝不得写入绑定");
    }
    assert_eq!(
        count_ops(&ctx),
        ops_baseline,
        "--dry-run 拒绝不得新增操作审计记录"
    );

    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn skills_discovery_error_surfaces_on_collection_and_recovers() {
    let (root, context) = temp_ctx("skills-discovery-error");

    // 受管仓库缓存已落地，但 skill_paths 指向不存在的目录 ghost
    let repo_dir = context.data_dir.join("repositories").join("broken-skills");
    std::fs::create_dir_all(repo_dir.join("alpha")).unwrap();
    std::fs::write(repo_dir.join("alpha").join("SKILL.md"), "# Alpha\n").unwrap();

    let set_res = execute(
        &context,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.broken-skills",
                "--value",
                "{\"url\":\"https://example.com/broken-skills.git\",\"skill_paths\":[\"alpha\",\"ghost\"],\"agents\":[\"codex\"]}",
            ],
        ),
    );
    assert!(set_res.ok, "{:?}", set_res.errors);

    // 1. 发现失败：status 信封仍 ok，集合元素带 discovery_error 且 discovered 为空
    let status = execute(&context, &request("skills:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    assert!(status.errors.is_empty(), "status 不应产生错误诊断");
    let elements = status.data["elements"].as_array().unwrap();
    let coll_el = elements
        .iter()
        .find(|el| el["id"] == "skill:broken-skills")
        .expect("集合元素必须存在");
    let discovery_error = &coll_el["observed"]["discovery_error"];
    assert_eq!(discovery_error["category"], "missing-dir");
    assert_eq!(discovery_error["skill_paths_item"], "ghost");
    let message = discovery_error["message"].as_str().unwrap();
    assert!(
        message.contains("missing-dir") && message.contains("ghost"),
        "消息须含失败类别与问题项: {}",
        message
    );
    assert_eq!(
        coll_el["observed"]["discovered"].as_array().unwrap().len(),
        0,
        "发现失败时 discovered 必须为空"
    );
    let blocked = coll_el["observed"]["blocked_reason"].as_str().unwrap();
    assert!(
        blocked.contains("missing-dir"),
        "受阻摘要须可读: {}",
        blocked
    );
    // state 计算保持不变：仓库已缓存且启用 → managed-ok；actions 不变
    assert_eq!(coll_el["state"], "managed-ok");
    let actions: Vec<&str> = coll_el["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert_eq!(actions, vec!["update", "disable", "remove"]);
    // 集合下不产生任何 skill 元素
    assert!(
        !elements.iter().any(|el| el["id"]
            .as_str()
            .unwrap_or("")
            .starts_with("skill:broken-skills/")),
        "发现失败时不应产生 skill 元素"
    );

    // 2. 修复（目录补齐）后 discovery_error 消失，技能正常发现
    std::fs::create_dir_all(repo_dir.join("ghost")).unwrap();
    std::fs::write(repo_dir.join("ghost").join("SKILL.md"), "# Ghost\n").unwrap();
    let status2 = execute(&context, &request("skills:status", &[]));
    assert!(status2.ok, "{:?}", status2.errors);
    let elements2 = status2.data["elements"].as_array().unwrap();
    let coll_el2 = elements2
        .iter()
        .find(|el| el["id"] == "skill:broken-skills")
        .unwrap();
    assert!(coll_el2["observed"]["discovery_error"].is_null());
    assert!(coll_el2["observed"]["blocked_reason"].is_null());
    let discovered: Vec<&str> = coll_el2["observed"]["discovered"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert_eq!(discovered, vec!["alpha", "ghost"]);

    let _ = std::fs::remove_dir_all(root);
}

fn set_collection(context: &Context, name: &str, value: &str) {
    let res = execute(
        context,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                &format!("skills.repositories.{name}"),
                "--value",
                value,
            ],
        ),
    );
    assert!(res.ok, "{:?}", res.errors);
}

fn collection_discovery(context: &Context, name: &str) -> (Vec<String>, serde_json::Value) {
    let status = execute(context, &request("skills:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    let coll = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|el| el["id"] == format!("skill:{name}"))
        .expect("集合元素必须存在")
        .clone();
    let discovered = coll["observed"]["discovered"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect();
    (discovered, coll["observed"]["discovery_error"].clone())
}

#[test]
fn skills_moved_under_skills_dir_are_found_by_old_and_new_definitions() {
    let (root, context) = temp_ctx("skills-moved-layout");
    // 来源把技能收进了 skills/，顶层只剩规则等非技能目录
    let repo_dir = context.data_dir.join("repositories").join("moved-skills");
    for name in ["alpha", "beta"] {
        std::fs::create_dir_all(repo_dir.join("skills").join(name)).unwrap();
        std::fs::write(
            repo_dir.join("skills").join(name).join("SKILL.md"),
            format!("# {name}\n"),
        )
        .unwrap();
    }
    std::fs::create_dir_all(repo_dir.join("rules")).unwrap();
    std::fs::write(repo_dir.join("rules").join("core.md"), "# core\n").unwrap();

    // 1. 旧定义列的是顶层技能名：到 skills/<名> 找到，照常发现
    set_collection(
        &context,
        "moved-skills",
        r#"{"url":"https://example.com/moved-skills.git","skill_paths":["alpha","beta"],"agents":["codex"]}"#,
    );
    let (discovered, error) = collection_discovery(&context, "moved-skills");
    assert!(error.is_null(), "旧定义不应报发现失败: {error}");
    assert_eq!(discovered, ["alpha", "beta"]);

    // 按旧定义启用：链接到的快照内容就是 skills/alpha 的内容
    let apply = execute(
        &context,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:moved-skills/alpha",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(apply.ok, "{:?}", apply.errors);
    let installed = context.home_dir.join(".codex/skills/alpha/SKILL.md");
    assert_eq!(std::fs::read_to_string(installed).unwrap(), "# alpha\n");

    // 2. 新定义直接取 skills/ 目录：发现同一组技能，不把 rules/ 当技能
    set_collection(
        &context,
        "moved-skills",
        r#"{"url":"https://example.com/moved-skills.git","skill_paths":["skills"],"agents":["codex"]}"#,
    );
    let (discovered, error) = collection_discovery(&context, "moved-skills");
    assert!(error.is_null(), "{error}");
    assert_eq!(discovered, ["alpha", "beta"]);

    // 3. 只有单段技能名才回退；多段路径缺失照旧报目录不存在
    set_collection(
        &context,
        "moved-skills",
        r#"{"url":"https://example.com/moved-skills.git","skill_paths":["nested/alpha"],"agents":["codex"]}"#,
    );
    let (discovered, error) = collection_discovery(&context, "moved-skills");
    assert_eq!(error["category"], "missing-dir");
    assert!(discovered.is_empty());

    let _ = std::fs::remove_dir_all(root);
}

fn skill_cell(context: &Context, id: &str, agent: &str) -> Option<serde_json::Value> {
    let status = execute(context, &request("skills:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|el| el["id"] == id)
        .map(|el| el["observed"]["agents"][agent].clone())
}

fn binding_agents(context: &Context, coll: &str, skill: &str) -> Vec<String> {
    let conn = rusqlite::Connection::open(context.data_dir.join("native-manager.db")).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT agent FROM skill_bindings WHERE collection = ?1 AND skill = ?2 ORDER BY agent",
        )
        .unwrap();
    stmt.query_map([coll, skill], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn skill_action(
    context: &Context,
    id: &str,
    action: &str,
    agent: &str,
) -> agent_manager_engine::Envelope {
    execute(
        context,
        &request(
            "skills:apply",
            &["--only", id, "--action", action, "--agent", agent],
        ),
    )
}

#[test]
fn skill_dropped_by_its_source_can_only_be_removed_and_leaves_no_record() {
    let (root, context) = temp_ctx("skills-dropped-upstream");
    let repo_dir = context.data_dir.join("repositories").join("drop-skills");
    for name in ["alpha", "beta"] {
        std::fs::create_dir_all(repo_dir.join("skills").join(name)).unwrap();
        std::fs::write(
            repo_dir.join("skills").join(name).join("SKILL.md"),
            format!("# {name}\n"),
        )
        .unwrap();
    }
    set_collection(
        &context,
        "drop-skills",
        r#"{"url":"https://example.com/drop-skills.git","skill_paths":["skills"]}"#,
    );
    for agent in ["claude-code", "codex"] {
        let add = skill_action(&context, "skill:drop-skills/beta", "add", agent);
        assert!(add.ok, "{:?}", add.errors);
    }
    let claude_link = context.home_dir.join(".claude/skills/beta");
    let codex_link = context.home_dir.join(".codex/skills/beta");

    // 上游删掉 beta；codex 那条链接也已经没了，claude 的还指着旧快照
    std::fs::remove_dir_all(repo_dir.join("skills").join("beta")).unwrap();
    agent_manager_engine::platform::remove_link(&codex_link).unwrap();
    for (agent, state) in [("claude-code", "linked"), ("codex", "missing-link")] {
        let cell = skill_cell(&context, "skill:drop-skills/beta", agent).unwrap();
        assert_eq!(cell["state"], state, "{agent}: {cell}");
        assert_eq!(cell["source_missing"], true, "{agent}: {cell}");
        assert_eq!(
            cell["actions"],
            serde_json::json!(["remove"]),
            "{agent}: {cell}"
        );
        assert!(
            cell["blocked_reason"]
                .as_str()
                .unwrap()
                .contains("来源里已没有"),
            "{agent}: {cell}"
        );
    }
    // 来源里还在的技能不受影响
    let alpha = skill_cell(&context, "skill:drop-skills/alpha", "codex").unwrap();
    assert_eq!(alpha["source_missing"], false);

    // 停用做不成：照旧拒绝，记录不动
    let disable = skill_action(&context, "skill:drop-skills/beta", "disable", "codex");
    assert!(!disable.ok);
    assert_eq!(
        binding_agents(&context, "drop-skills", "beta"),
        ["claude-code", "codex"]
    );

    // 逐助手移除：codex 的记录删掉，claude 的记录与链接不动
    let remove = skill_action(&context, "skill:drop-skills/beta", "remove", "codex");
    assert!(remove.ok, "{:?}", remove.errors);
    assert_eq!(
        binding_agents(&context, "drop-skills", "beta"),
        ["claude-code"]
    );
    assert!(
        std::fs::symlink_metadata(&claude_link).is_ok(),
        "另一个助手的链接不能动"
    );
    let codex_cell = skill_cell(&context, "skill:drop-skills/beta", "codex").unwrap();
    assert_eq!(codex_cell["state"], "not-assigned");
    assert_eq!(codex_cell["source_missing"], false);

    // claude 那条链接还在（本工具建的），移除时一并删掉；最后这个技能从状态里消失
    let remove = skill_action(&context, "skill:drop-skills/beta", "remove", "claude-code");
    assert!(remove.ok, "{:?}", remove.errors);
    assert!(
        std::fs::symlink_metadata(&claude_link).is_err(),
        "残留链接应删掉"
    );
    assert!(binding_agents(&context, "drop-skills", "beta").is_empty());
    assert!(skill_cell(&context, "skill:drop-skills/beta", "codex").is_none());

    // 没有分配记录的也不能凭空「移除」
    let again = skill_action(&context, "skill:drop-skills/beta", "remove", "codex");
    assert!(!again.ok);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unfetched_or_broken_source_is_not_mistaken_for_a_dropped_skill() {
    let (root, context) = temp_ctx("skills-source-unknown");
    let repo_dir = context.data_dir.join("repositories").join("flaky-skills");
    std::fs::create_dir_all(repo_dir.join("skills").join("alpha")).unwrap();
    std::fs::write(repo_dir.join("skills/alpha/SKILL.md"), "# alpha\n").unwrap();
    set_collection(
        &context,
        "flaky-skills",
        r#"{"url":"https://example.com/flaky-skills.git","skill_paths":["skills"]}"#,
    );
    let add = skill_action(&context, "skill:flaky-skills/alpha", "add", "codex");
    assert!(add.ok, "{:?}", add.errors);

    // 发现失败：技能都找不到，但不能提示移除
    set_collection(
        &context,
        "flaky-skills",
        r#"{"url":"https://example.com/flaky-skills.git","skill_paths":["skills","ghost"]}"#,
    );
    let cell = skill_cell(&context, "skill:flaky-skills/alpha", "codex").unwrap();
    assert_eq!(cell["source_missing"], false, "{cell}");
    assert_ne!(cell["actions"], serde_json::json!(["remove"]), "{cell}");

    // 镜像不在：同样不算上游删掉
    set_collection(
        &context,
        "flaky-skills",
        r#"{"url":"https://example.com/flaky-skills.git","skill_paths":["skills"]}"#,
    );
    std::fs::rename(&repo_dir, context.data_dir.join("moved-away")).unwrap();
    let cell = skill_cell(&context, "skill:flaky-skills/alpha", "codex").unwrap();
    assert_eq!(cell["source_missing"], false, "{cell}");

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn skills_dir_fallback_does_not_escape_the_repository() {
    let (root, context) = temp_ctx("skills-moved-escape");
    let repo_dir = context.data_dir.join("repositories").join("escape-skills");
    std::fs::create_dir_all(&repo_dir).unwrap();
    // 仓库里的 skills 是指向仓库外的链接：回退找到的目录也必须按越界拒绝
    let outside = root.join("outside");
    std::fs::create_dir_all(outside.join("alpha")).unwrap();
    std::fs::write(outside.join("alpha").join("SKILL.md"), "# outside\n").unwrap();
    std::os::unix::fs::symlink(&outside, repo_dir.join("skills")).unwrap();

    set_collection(
        &context,
        "escape-skills",
        r#"{"url":"https://example.com/escape-skills.git","skill_paths":["alpha"],"agents":["codex"]}"#,
    );
    let (discovered, error) = collection_discovery(&context, "escape-skills");
    assert_eq!(error["category"], "out-of-bounds");
    assert!(discovered.is_empty());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn skills_extra_link_takeover_and_unlink() {
    let (base, ctx) = temp_ctx("skills-extra-link");
    let repo_dir = ctx.data_dir.join("repositories").join("takeover-coll");
    std::fs::create_dir_all(repo_dir.join("alpha")).unwrap();
    std::fs::write(repo_dir.join("alpha").join("SKILL.md"), "# alpha").unwrap();
    std::fs::create_dir_all(repo_dir.join("beta")).unwrap();
    std::fs::write(repo_dir.join("beta").join("SKILL.md"), "# beta").unwrap();

    let set = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.takeover-coll",
                "--value",
                "{\"url\":\"https://example.com/takeover.git\",\"skill_paths\":[\"alpha\",\"beta\"],\"agents\":[\"codex\"]}",
            ],
        ),
    );
    assert!(set.ok, "{:?}", set.errors);

    // 旧体系遗留：codex 技能目录下的外部符号链接（目标在受管基目录之外）
    let external_target = ctx.home_dir.join("external-skills").join("alpha");
    std::fs::create_dir_all(&external_target).unwrap();
    std::fs::write(external_target.join("SKILL.md"), "# external alpha").unwrap();
    let link_path = ctx.home_dir.join(".codex").join("skills").join("alpha");
    std::fs::create_dir_all(link_path.parent().unwrap()).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&external_target, &link_path).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&external_target, &link_path).unwrap();

    // 1. status：extra-link 行提供 add+remove，managed=false，version 为空
    let st = execute(&ctx, &request("skills:status", &[]));
    assert!(st.ok, "{:?}", st.errors);
    let alpha_el = st.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|el| el["id"] == "skill:takeover-coll/alpha")
        .unwrap();
    let cell = &alpha_el["observed"]["agents"]["codex"];
    assert_eq!(cell["state"], "extra-link");
    assert_eq!(
        cell["actions"],
        serde_json::json!(["add", "remove"]),
        "计划外链接行必须提供 纳入管理(add) 与 取消链接(remove)"
    );
    assert_eq!(cell["managed"], false, "外部链接必须标记为非受管");
    assert!(cell["version"].is_null());

    // 2. 缺旗标 fail-closed：add 与 remove 均拒绝，链接与目标内容保持原样、绑定零写
    let add_no_flag = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:takeover-coll/alpha",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(
        !add_no_flag.ok,
        "非受管链接缺 --allow-takeover 的 add 必须拒绝"
    );
    assert_eq!(add_no_flag.errors[0].code, "SKILL_ERROR");
    assert!(add_no_flag.errors[0].message.contains("外部非受管链接"));
    assert!(add_no_flag.errors[0].message.contains("--allow-takeover"));

    let rm_no_flag = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:takeover-coll/alpha",
                "--action",
                "remove",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(
        !rm_no_flag.ok,
        "非受管链接缺 --allow-takeover 的 remove 必须拒绝"
    );
    assert!(rm_no_flag.errors[0].message.contains("--allow-takeover"));
    assert_eq!(
        std::fs::read_link(&link_path).unwrap(),
        external_target,
        "被拒后外部链接必须保持原样"
    );
    assert!(
        external_target.join("SKILL.md").exists(),
        "被拒后外部目标内容必须完好"
    );
    if let Some(conn) = store::open_readonly(&ctx).unwrap() {
        let count: i64 = conn
            .query_row("SELECT count(*) FROM skill_bindings", [], |r| r.get(0))
            .unwrap_or(0);
        assert_eq!(count, 0, "preflight 拒绝不得写入绑定");
    }

    // 3. 旗标适用范围：update/enable/disable 与集合级动作携带 --allow-takeover 一律拒绝
    for action in ["update", "enable", "disable"] {
        let res = execute(
            &ctx,
            &request(
                "skills:apply",
                &[
                    "--only",
                    "skill:takeover-coll/alpha",
                    "--action",
                    action,
                    "--agent",
                    "codex",
                    "--allow-takeover",
                ],
            ),
        );
        assert!(!res.ok, "{action} 携带 --allow-takeover 必须拒绝");
        assert!(
            res.errors[0]
                .message
                .contains("仅适用于逐 skill 的 add/remove"),
            "{action} 拒绝消息须说明旗标适用范围: {}",
            res.errors[0].message
        );
    }
    let coll_flag = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:takeover-coll",
                "--action",
                "remove",
                "--allow-takeover",
            ],
        ),
    );
    assert!(!coll_flag.ok, "集合级动作携带 --allow-takeover 必须拒绝");
    assert!(coll_flag.errors[0]
        .message
        .contains("仅适用于逐 skill 的 add/remove"));

    // 4. 带旗标 remove（取消链接）：外部链接被移除，链接目标内容保留，审计 takeover=true
    let rm = execute(
        &ctx,
        &request_with_run(
            "skills:apply",
            &[
                "--only",
                "skill:takeover-coll/alpha",
                "--action",
                "remove",
                "--agent",
                "codex",
                "--allow-takeover",
            ],
            "extra-unlink-run",
        ),
    );
    assert!(rm.ok, "{:?}", rm.errors);
    assert!(
        std::fs::symlink_metadata(&link_path).is_err(),
        "取消链接后链接入口必须不存在"
    );
    assert!(
        external_target.join("SKILL.md").exists(),
        "取消链接绝不删除链接目标的内容"
    );
    let runs_rm = execute(
        &ctx,
        &request_with_run("runs:get", &["--run", "extra-unlink-run"], "audit-unlink"),
    );
    assert!(runs_rm.ok, "{:?}", runs_rm.errors);
    let rm_op = runs_rm.data["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["operation_type"] == "skills:remove")
        .expect("必须有 skills:remove 审计记录");
    assert_eq!(
        rm_op["detail"]["takeover"], true,
        "接管删除必须标记 takeover"
    );

    // 5. 重新放回外部链接，带旗标 add（纳入管理）：快照落地、原子替换、绑定写入
    #[cfg(unix)]
    std::os::unix::fs::symlink(&external_target, &link_path).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&external_target, &link_path).unwrap();
    let add = execute(
        &ctx,
        &request_with_run(
            "skills:apply",
            &[
                "--only",
                "skill:takeover-coll/alpha",
                "--action",
                "add",
                "--agent",
                "codex",
                "--allow-takeover",
            ],
            "extra-takeover-run",
        ),
    );
    assert!(add.ok, "{:?}", add.errors);
    let new_target = std::fs::read_link(&link_path).unwrap();
    assert!(
        new_target.to_string_lossy().contains("skill-versions"),
        "纳入管理后链接必须指向受管快照目录"
    );
    let st_after = execute(&ctx, &request("skills:status", &[]));
    let cell_after = &st_after.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|el| el["id"] == "skill:takeover-coll/alpha")
        .unwrap()["observed"]["agents"]["codex"];
    assert_eq!(cell_after["state"], "linked");
    assert_eq!(cell_after["assigned"], true);
    assert_eq!(cell_after["managed"], true);
    let runs_add = execute(
        &ctx,
        &request_with_run(
            "runs:get",
            &["--run", "extra-takeover-run"],
            "audit-takeover",
        ),
    );
    assert!(runs_add.ok, "{:?}", runs_add.errors);
    let add_op = runs_add.data["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["operation_type"] == "skills:add")
        .expect("必须有 skills:add 审计记录");
    assert_eq!(
        add_op["detail"]["takeover"], true,
        "接管替换必须标记 takeover"
    );

    // 6. 无冲突目标即使带旗标也不算接管：beta 正常 add，审计 takeover=false
    let add_clean = execute(
        &ctx,
        &request_with_run(
            "skills:apply",
            &[
                "--only",
                "skill:takeover-coll/beta",
                "--action",
                "add",
                "--agent",
                "codex",
                "--allow-takeover",
            ],
            "extra-clean-run",
        ),
    );
    assert!(add_clean.ok, "{:?}", add_clean.errors);
    let runs_clean = execute(
        &ctx,
        &request_with_run("runs:get", &["--run", "extra-clean-run"], "audit-clean"),
    );
    let clean_op = runs_clean.data["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["operation_type"] == "skills:add")
        .expect("必须有 beta 的 skills:add 审计记录");
    assert_eq!(
        clean_op["detail"]["takeover"], false,
        "无非受管链接时审计不得标记 takeover"
    );

    // 7. 受管残留链接：绑定已移除（assigned=false）但受管链接仍在 → extra-link 且 managed=true，
    //    remove 无需 takeover 旗标即可清理，审计 takeover=false
    {
        let conn = store::open_write(&ctx).unwrap();
        conn.execute(
            "UPDATE skill_bindings SET assigned = 0, enabled = 0 WHERE collection = 'takeover-coll' AND skill = 'alpha' AND agent = 'codex'",
            [],
        )
        .unwrap();
    }
    let st_leftover = execute(&ctx, &request("skills:status", &[]));
    let cell_leftover = &st_leftover.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|el| el["id"] == "skill:takeover-coll/alpha")
        .unwrap()["observed"]["agents"]["codex"];
    assert_eq!(cell_leftover["state"], "extra-link");
    assert_eq!(cell_leftover["managed"], true, "受管残留链接必须标记为受管");
    assert_eq!(
        cell_leftover["actions"],
        serde_json::json!(["add", "remove"])
    );

    let rm_managed = execute(
        &ctx,
        &request_with_run(
            "skills:apply",
            &[
                "--only",
                "skill:takeover-coll/alpha",
                "--action",
                "remove",
                "--agent",
                "codex",
            ],
            "extra-managed-rm-run",
        ),
    );
    assert!(
        rm_managed.ok,
        "受管残留链接的 remove 无需旗标: {:?}",
        rm_managed.errors
    );
    assert!(std::fs::symlink_metadata(&link_path).is_err());
    let runs_managed_rm = execute(
        &ctx,
        &request_with_run(
            "runs:get",
            &["--run", "extra-managed-rm-run"],
            "audit-managed-rm",
        ),
    );
    let managed_rm_op = runs_managed_rm.data["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["operation_type"] == "skills:remove")
        .expect("必须有受管残留的 skills:remove 审计记录");
    assert_eq!(managed_rm_op["detail"]["takeover"], false);

    let _ = std::fs::remove_dir_all(base);
}

// ---------- 本地来源集合（source_path 直链接管） ----------

/// 用户直接指定到某个技能文件夹（里面就是 SKILL.md）或更深的子目录：链接指向那个文件夹本身，
/// 不拼成「源根/技能名」。
#[test]
fn local_source_can_point_at_a_single_skill_folder_or_nested_path() {
    let (root, context) = temp_ctx("skills-local-single");
    let solo = context.home_dir.join("solo").join("gamma");
    std::fs::create_dir_all(&solo).unwrap();
    std::fs::write(solo.join("SKILL.md"), "# Gamma").unwrap();
    let packs = context.home_dir.join("packs").join("team").join("delta");
    std::fs::create_dir_all(&packs).unwrap();
    std::fs::write(packs.join("SKILL.md"), "# Delta").unwrap();
    std::fs::create_dir_all(context.home_dir.join(".claude").join("skills")).unwrap();
    for (name, def) in [
        ("gamma", r#"{"source_path":"~/solo/gamma"}"#),
        (
            "packs",
            r#"{"source_path":"~/packs","skill_paths":["team/delta"]}"#,
        ),
    ] {
        let key = format!("skills.repositories.{name}");
        let set = execute(
            &context,
            &request(
                "policy:set",
                &["--layer", "local", "--key", &key, "--value", def],
            ),
        );
        assert!(set.ok, "{:?}", set.errors);
    }
    for (only, expected) in [("gamma/gamma", &solo), ("packs/delta", &packs)] {
        let add = execute(
            &context,
            &request(
                "skills:apply",
                &["--only", only, "--agent", "claude-code", "--action", "add"],
            ),
        );
        assert!(add.ok, "{only}: {:?}", add.errors);
        let name = only.rsplit('/').next().unwrap();
        let link = context.home_dir.join(".claude").join("skills").join(name);
        assert_eq!(
            std::fs::read_link(&link).unwrap().canonicalize().unwrap(),
            expected.canonicalize().unwrap(),
            "{only}"
        );
    }
    let status = execute(&context, &request("skills:status", &[]));
    for id in ["skill:gamma/gamma", "skill:packs/delta"] {
        let element = status.data["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|el| el["id"] == id)
            .unwrap();
        assert_eq!(
            element["observed"]["agents"]["claude-code"]["state"], "linked",
            "{id}"
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

/// 本地来源集合的端到端接管：登记外部源目录后，add 直接把助手目录里的链接指向源，
/// 内容跟随外部更新；停用/移除只动链接，绝不触碰外部源。
#[test]
fn local_source_collection_links_directly_to_external_source() {
    let (root, context) = temp_ctx("skills-local-source");
    // 外部工具自己维护的技能目录（manager 不拥有）
    let vendor = context.home_dir.join(".vendor-tool").join("skills");
    std::fs::create_dir_all(vendor.join("alpha")).unwrap();
    std::fs::write(vendor.join("alpha").join("SKILL.md"), "# Alpha\nv1").unwrap();
    // 接管前就已经存在的外部链接（旧体系/手工建立）
    let claude_skills = context.home_dir.join(".claude").join("skills");
    std::fs::create_dir_all(&claude_skills).unwrap();
    link_dir(vendor.join("alpha"), claude_skills.join("alpha")).unwrap();

    let set_res = execute(
        &context,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.vendor-tool",
                "--value",
                "{\"source_path\":\"~/.vendor-tool/skills\",\"skill_paths\":[\"alpha\"],\"agents\":[\"claude-code\"]}",
            ],
        ),
    );
    assert!(set_res.ok, "{:?}", set_res.errors);

    // status：集合以源目录为资源库，且不冒充 manager 自管缓存
    let status = execute(&context, &request("skills:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    let coll_el = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|el| el["id"] == "skill:vendor-tool")
        .unwrap()
        .clone();
    assert_eq!(coll_el["spec"]["source_path"], "~/.vendor-tool/skills");
    assert_eq!(coll_el["spec"]["url"], serde_json::Value::Null);
    assert_eq!(coll_el["state"], "managed-ok");
    assert_eq!(coll_el["observed"]["owner_marked"], false);

    // add：接管既有外部链接（目标不变，文件系统零改动），不产生内容快照
    let add = execute(
        &context,
        &request(
            "skills:apply",
            &[
                "--only",
                "vendor-tool/alpha",
                "--agent",
                "claude-code",
                "--action",
                "add",
            ],
        ),
    );
    assert!(add.ok, "{:?}", add.errors);
    let link = claude_skills.join("alpha");
    // 链接目标经词法/真实路径归一（macOS /var → /private/var），比较解析后的同一目录。
    let links_to_source = |link: &std::path::Path| {
        std::fs::read_link(link).unwrap().canonicalize().unwrap()
            == vendor.join("alpha").canonicalize().unwrap()
    };
    assert!(links_to_source(&link), "本地来源集合必须直链源目录");
    assert!(
        !context.data_dir.join("skill-versions").exists(),
        "直链模式不得复制内容快照"
    );

    // 内容跟随外部更新（无需 update）
    std::fs::write(vendor.join("alpha").join("SKILL.md"), "# Alpha\nv2").unwrap();
    assert_eq!(
        std::fs::read_to_string(link.join("SKILL.md")).unwrap(),
        "# Alpha\nv2"
    );

    let linked_state = |env: &agent_manager_engine::Envelope| -> String {
        env.data["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|el| el["id"] == "skill:vendor-tool/alpha")
            .unwrap()["observed"]["agents"]["claude-code"]["state"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let after_add = execute(&context, &request("skills:status", &[]));
    assert_eq!(linked_state(&after_add), "linked");

    // disable：只删链接，外部源原样保留
    let disable = execute(
        &context,
        &request(
            "skills:apply",
            &[
                "--only",
                "vendor-tool/alpha",
                "--agent",
                "claude-code",
                "--action",
                "disable",
            ],
        ),
    );
    assert!(disable.ok, "{:?}", disable.errors);
    assert!(!link.exists() && std::fs::read_link(&link).is_err());
    assert!(
        vendor.join("alpha").join("SKILL.md").is_file(),
        "停用不得删除外部源内容"
    );

    // enable：重新直链回源
    let enable = execute(
        &context,
        &request(
            "skills:apply",
            &[
                "--only",
                "vendor-tool/alpha",
                "--agent",
                "claude-code",
                "--action",
                "enable",
            ],
        ),
    );
    assert!(enable.ok, "{:?}", enable.errors);
    assert!(links_to_source(&link));

    // 集合级 update：无网络、无 clone，刷新指纹并保持直链
    let update = execute(
        &context,
        &request(
            "skills:apply",
            &["--only", "vendor-tool", "--action", "update"],
        ),
    );
    assert!(update.ok, "{:?}", update.errors);
    assert!(links_to_source(&link));
    assert!(!context
        .data_dir
        .join("repositories")
        .join("vendor-tool")
        .exists());

    // remove：解除管理，外部源仍在
    let remove = execute(
        &context,
        &request(
            "skills:apply",
            &[
                "--only",
                "vendor-tool/alpha",
                "--agent",
                "claude-code",
                "--action",
                "remove",
            ],
        ),
    );
    assert!(remove.ok, "{:?}", remove.errors);
    assert!(vendor.join("alpha").join("SKILL.md").is_file());

    let _ = std::fs::remove_dir_all(root);
}

/// 本地来源集合的源目录安全反例：非法定义一律 fail-closed，且不留下任何链接。
#[test]
fn local_source_rejects_unsafe_definitions() {
    let (root, context) = temp_ctx("skills-local-source-guard");
    let vendor = context.home_dir.join(".vendor-tool").join("skills");
    std::fs::create_dir_all(vendor.join("alpha")).unwrap();
    std::fs::write(vendor.join("alpha").join("SKILL.md"), "# Alpha").unwrap();
    std::fs::create_dir_all(context.home_dir.join(".claude").join("skills")).unwrap();
    // 指向源目录的符号链接（不允许作为 source_path 根）
    let linked_root = context.home_dir.join("linked-skills");
    link_dir(&vendor, &linked_root).unwrap();

    let cases: Vec<(&str, String)> = vec![
        (
            "url 与 source_path 互斥",
            "{\"source_path\":\"~/.vendor-tool/skills\",\"url\":\"https://example.com/x.git\",\"skill_paths\":[\"alpha\"]}".to_string(),
        ),
        (
            "源目录不存在",
            "{\"source_path\":\"~/.missing-tool/skills\",\"skill_paths\":[\"alpha\"]}".to_string(),
        ),
        (
            "源根是符号链接",
            "{\"source_path\":\"~/linked-skills\",\"skill_paths\":[\"alpha\"]}".to_string(),
        ),
        (
            "源落在助手技能目录内（自指）",
            "{\"source_path\":\"~/.claude/skills\",\"skill_paths\":[\"alpha\"]}".to_string(),
        ),
    ];

    for (label, definition) in cases {
        let set_res = execute(
            &context,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "skills.repositories.guarded",
                    "--value",
                    &definition,
                ],
            ),
        );
        assert!(set_res.ok, "保存定义本身不执行动作: {label}");
        let add = execute(
            &context,
            &request(
                "skills:apply",
                &[
                    "--only",
                    "guarded/alpha",
                    "--agent",
                    "claude-code",
                    "--action",
                    "add",
                ],
            ),
        );
        assert!(!add.ok, "[{label}] 必须 fail-closed 拒绝");
        assert_eq!(add.errors[0].code, "SKILL_ERROR", "[{label}]");
        assert!(
            !context.home_dir.join(".claude/skills/alpha").exists(),
            "[{label}] 失败不得留下链接"
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

/// 本地来源集合的漂移保护：链接被外部改指到别处时 status 报 drifted，
/// 且不被静默覆盖（enable 会按期望目标重建，但外部真实目录仍 fail-closed）。
#[test]
fn local_source_detects_drifted_link() {
    let (root, context) = temp_ctx("skills-local-drift");
    let vendor = context.home_dir.join(".vendor-tool").join("skills");
    std::fs::create_dir_all(vendor.join("alpha")).unwrap();
    std::fs::write(vendor.join("alpha").join("SKILL.md"), "# Alpha").unwrap();
    let other = context.home_dir.join("other-place");
    std::fs::create_dir_all(&other).unwrap();
    let claude_skills = context.home_dir.join(".claude").join("skills");
    std::fs::create_dir_all(&claude_skills).unwrap();

    let set_res = execute(
        &context,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.vendor-tool",
                "--value",
                "{\"source_path\":\"~/.vendor-tool/skills\",\"skill_paths\":[\"alpha\"],\"agents\":[\"claude-code\"]}",
            ],
        ),
    );
    assert!(set_res.ok, "{:?}", set_res.errors);
    let add = execute(
        &context,
        &request(
            "skills:apply",
            &[
                "--only",
                "vendor-tool/alpha",
                "--agent",
                "claude-code",
                "--action",
                "add",
            ],
        ),
    );
    assert!(add.ok, "{:?}", add.errors);

    // 外部把链接改指到别处
    let link = claude_skills.join("alpha");
    agent_manager_engine::platform::remove_link(&link).unwrap();
    link_dir(&other, &link).unwrap();

    let status = execute(&context, &request("skills:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    let cell = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|el| el["id"] == "skill:vendor-tool/alpha")
        .unwrap()["observed"]["agents"]["claude-code"]["state"]
        .clone();
    assert_eq!(cell, "drifted", "改指到登记源之外必须报漂移");

    // 真实目录占位时 fail-closed：不接管、不覆盖
    agent_manager_engine::platform::remove_link(&link).unwrap();
    std::fs::create_dir_all(link.join("inner")).unwrap();
    let enable = execute(
        &context,
        &request(
            "skills:apply",
            &[
                "--only",
                "vendor-tool/alpha",
                "--agent",
                "claude-code",
                "--action",
                "enable",
            ],
        ),
    );
    assert!(!enable.ok, "真实目录占位必须 fail-closed");
    assert!(link.join("inner").is_dir(), "外部真实目录不得被覆盖");
    let _ = std::fs::remove_dir_all(root);
}

// ---------- 孤立链接清理（remove-link） ----------

/// 断链与旧体系残留：只删链接本身，真实目录与有分配记录的条目一律拒绝。
#[test]
fn remove_link_cleans_orphans_and_refuses_the_rest() {
    let (root, context) = temp_ctx("skills-remove-link");
    let claude_skills = context.home_dir.join(".claude").join("skills");
    std::fs::create_dir_all(&claude_skills).unwrap();

    // 1) 断链
    link_dir(
        context.home_dir.join("gone/ghost"),
        claude_skills.join("ghost"),
    )
    .unwrap();
    // 2) 指向真实外部目录的链接（源必须完好保留）
    let vendor = context.home_dir.join(".vendor-tool").join("skills");
    std::fs::create_dir_all(vendor.join("alpha")).unwrap();
    std::fs::write(vendor.join("alpha").join("SKILL.md"), "# Alpha").unwrap();
    link_dir(vendor.join("alpha"), claude_skills.join("alpha")).unwrap();
    // 3) 真实目录（不是链接）
    std::fs::create_dir_all(claude_skills.join("synced")).unwrap();

    let remove = |link: &str| {
        execute(
            &context,
            &request(
                "skills:apply",
                &[
                    "--action",
                    "remove-link",
                    "--agent",
                    "claude-code",
                    "--link",
                    link,
                ],
            ),
        )
    };

    let ghost = remove("ghost");
    assert!(ghost.ok, "{:?}", ghost.errors);
    assert!(
        std::fs::symlink_metadata(claude_skills.join("ghost")).is_err(),
        "断链应已清理"
    );

    let alpha = remove("alpha");
    assert!(alpha.ok, "{:?}", alpha.errors);
    assert!(std::fs::symlink_metadata(claude_skills.join("alpha")).is_err());
    assert!(
        vendor.join("alpha").join("SKILL.md").is_file(),
        "只删链接，绝不跟随链接删源"
    );

    let real_dir = remove("synced");
    assert!(!real_dir.ok, "真实目录必须 fail-closed");
    assert!(claude_skills.join("synced").is_dir(), "拒绝时目录原样保留");

    let missing = remove("not-there");
    assert!(!missing.ok, "不存在的目标应明确拒绝而不是谎报成功");

    // 4) 有分配记录的条目不能从清理入口绕过
    let conn = store::open_write(&context).unwrap();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS skill_bindings (
            collection TEXT NOT NULL, skill TEXT NOT NULL, agent TEXT NOT NULL,
            version TEXT NOT NULL, enabled INTEGER NOT NULL, assigned INTEGER NOT NULL,
            updated_at TEXT NOT NULL, PRIMARY KEY (collection, skill, agent));",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO skill_bindings VALUES ('vendor','bound','claude-code','v1',1,1,'now');",
        [],
    )
    .unwrap();
    drop(conn);
    link_dir(vendor.join("alpha"), claude_skills.join("bound")).unwrap();
    let bound = remove("bound");
    assert!(!bound.ok, "有分配记录的条目要走集合 remove");
    assert!(
        std::fs::symlink_metadata(claude_skills.join("bound")).is_ok(),
        "拒绝时链接保留"
    );

    // 参数反例
    for args in [
        &["--action", "remove-link", "--agent", "claude-code"][..],
        &["--action", "remove-link", "--link", "ghost"][..],
        &[
            "--action",
            "remove-link",
            "--agent",
            "claude-code",
            "--link",
            "ghost",
            "--only",
            "x/y",
        ][..],
        &[
            "--action",
            "remove-link",
            "--agent",
            "claude-code",
            "--link",
            "../escape",
        ][..],
    ] {
        let bad = execute(&context, &request("skills:apply", args));
        assert!(!bad.ok, "参数 {:?} 必须被拒绝", args);
    }

    let _ = std::fs::remove_dir_all(root);
}

/// 上游改写历史（force push / rebase）后，"更新资源库"必须仍然成功。
///
/// 仓库缓存是工具自己 clone 的只读镜像，本地不产生提交，远端就是唯一真相。
/// 早期用 `git pull --ff-only`：上游一旦改写历史，同一批提交换了 SHA，本地与远端
/// 永久分叉，这个按钮就再也点不动了——真机上就是这么卡住的。
#[test]
fn 上游改写历史后更新资源库仍然成功() {
    let (root, context) = temp_ctx("skills-force-push");

    let git = |dir: &std::path::Path, args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .expect("git 可执行");
        assert!(
            out.status.success(),
            "git {args:?} 失败: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    // 远端 fixture：一个带 alpha 技能的仓库
    let origin = root.join("origin");
    std::fs::create_dir_all(origin.join("alpha")).unwrap();
    std::fs::write(origin.join("alpha").join("SKILL.md"), "# Alpha\nv1").unwrap();
    git(&origin, &["init", "-q", "-b", "main"]);
    git(&origin, &["add", "-A"]);
    git(&origin, &["commit", "-qm", "v1"]);

    let repo_dir = context.data_dir.join("repositories").join("forced");
    std::fs::create_dir_all(repo_dir.parent().unwrap()).unwrap();
    git(
        &root,
        &[
            "clone",
            "-q",
            &origin.to_string_lossy(),
            &repo_dir.to_string_lossy(),
        ],
    );

    let set = execute(
        &context,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.forced",
                "--value",
                // 用 json! 拼：Windows 路径里的反斜杠要转义
                &serde_json::json!({
                    "url": origin.to_string_lossy(),
                    "skill_paths": ["alpha"],
                    "agents": ["codex"],
                })
                .to_string(),
            ],
        ),
    );
    assert!(set.ok, "{:?}", set.errors);

    // 上游改写历史：同样的内容、不同的 SHA，本地与远端从此分叉
    git(&origin, &["commit", "-q", "--amend", "-m", "v1 (重写)"]);
    std::fs::write(origin.join("alpha").join("SKILL.md"), "# Alpha\nv2").unwrap();
    git(&origin, &["add", "-A"]);
    git(&origin, &["commit", "-qm", "v2"]);
    let upstream_head = git(&origin, &["rev-parse", "HEAD"]);

    let local_before = git(&repo_dir, &["rev-parse", "HEAD"]);
    assert_ne!(local_before, upstream_head, "前提：本地确实落在旧历史上");

    let env = execute(
        &context,
        &request(
            "skills:apply",
            &["--only", "skill:forced", "--action", "update"],
        ),
    );
    assert!(env.ok, "分叉后更新仍应成功：{:?}", env.errors);
    assert_eq!(
        git(&repo_dir, &["rev-parse", "HEAD"]),
        upstream_head,
        "镜像必须对齐到远端当前 HEAD"
    );
    assert_eq!(
        std::fs::read_to_string(repo_dir.join("alpha").join("SKILL.md")).unwrap(),
        "# Alpha\nv2",
        "工作区内容要跟着远端走"
    );

    let _ = std::fs::remove_dir_all(root);
}

// ---------- 内置技能仓库改为推荐来源 ----------

/// 旧版本 947bc09 起固化进库的 agent-skills 原值。
const LEGACY_BUILTIN_REPO: &str = r#"{"url":"https://github.com/ly87ing/agent-skills.git","skill_paths":["safe-merge-review","solution-shaping","change-discipline","verification","artifact-hygiene","reader-facing-writing","technical-diagramming"],"agents":["claude-code","codex"]}"#;

/// 模拟旧版本写下的库：内置集合与它的开关原样固化进 policy，另可带旧版本留下的
/// skill_bindings 行。全部在建库的同一连接里完成（新库初始化不跑迁移）。
fn seed_legacy_builtin(ctx: &Context, repo: serde_json::Value, bindings: &[(&str, &str, &str)]) {
    let conn = store::open_write(ctx).unwrap();
    let mut policy = store::policy_from_connection(&conn).unwrap();
    policy["skills"] = serde_json::json!({
        "agent-skills": {"enabled": true},
        "repositories": {"agent-skills": repo},
    });
    store::write_policy(&conn, &policy).unwrap();
    if bindings.is_empty() {
        return;
    }
    conn.execute(
        "CREATE TABLE IF NOT EXISTS skill_bindings (
            collection TEXT NOT NULL, skill TEXT NOT NULL, agent TEXT NOT NULL,
            version TEXT NOT NULL, enabled INTEGER NOT NULL, assigned INTEGER NOT NULL,
            updated_at TEXT NOT NULL, PRIMARY KEY (collection, skill, agent));",
        [],
    )
    .unwrap();
    for (collection, skill, agent) in bindings {
        conn.execute(
            "INSERT INTO skill_bindings VALUES (?1, ?2, ?3, 'v1', 1, 1, 'now')",
            [collection, skill, agent],
        )
        .unwrap();
    }
}

fn stored_skills_policy(ctx: &Context) -> serde_json::Value {
    let conn = store::open_readonly(ctx).unwrap().unwrap();
    store::policy_from_connection(&conn).unwrap()["skills"].clone()
}

fn retire_log_count(ctx: &Context) -> i64 {
    let conn = store::open_readonly(ctx).unwrap().unwrap();
    conn.query_row(
        "SELECT count(*) FROM operation_logs WHERE operation_type = 'skills:retire-builtin' AND target = 'agent-skills'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

fn unrelated_write(ctx: &Context) {
    let env = execute(
        ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "mcp.definitions.demo",
                "--value",
                r#"{"command":"node","args":["demo.js"]}"#,
            ],
        ),
    );
    assert!(env.ok, "{:?}", env.errors);
}

/// 旧库固化了原内置值且从未生效：读路径按推荐处理（不作为元素、零写）；任一写操作
/// 把它移出 policy 并记日志，已下载的仓库镜像保留；迁移幂等。
#[test]
fn legacy_builtin_without_bindings_is_recommendation_and_retired_on_write() {
    let (root, ctx) = temp_ctx("skills-legacy-retire");
    seed_legacy_builtin(
        &ctx,
        serde_json::from_str(LEGACY_BUILTIN_REPO).unwrap(),
        &[],
    );
    // 旧版本「全部同步」已 clone 下来的镜像
    let mirror = ctx.data_dir.join("repositories").join("agent-skills");
    std::fs::create_dir_all(mirror.join("verification")).unwrap();
    std::fs::write(mirror.join("verification").join("SKILL.md"), "# v").unwrap();
    // 同名外部链接：不得因旧内置集合被判成计划外链接
    let vendor = ctx.home_dir.join(".vendor").join("skills");
    std::fs::create_dir_all(vendor.join("verification")).unwrap();
    std::fs::write(vendor.join("verification").join("SKILL.md"), "# v").unwrap();
    let claude_skills = ctx.home_dir.join(".claude").join("skills");
    std::fs::create_dir_all(&claude_skills).unwrap();
    link_dir(
        vendor.join("verification"),
        claude_skills.join("verification"),
    )
    .unwrap();

    let db = store::db_path(&ctx);
    let bytes_before = std::fs::read(&db).unwrap();
    let status = execute(&ctx, &request("skills:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    assert!(
        !status.data["elements"]
            .as_array()
            .unwrap()
            .iter()
            .any(|el| el["id"].as_str().unwrap().starts_with("skill:agent-skills")),
        "未采用的旧内置集合不得作为元素: {}",
        status.data["elements"]
    );
    assert_eq!(status.data["recommendations"][0]["name"], "agent-skills");
    assert_eq!(status.data["recommendations"][0]["added"], false);
    let discover = execute(&ctx, &request("system:discover", &["--only", "skills"]));
    assert!(discover.ok, "{:?}", discover.errors);
    let item = discover.data["skills"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "verification")
        .unwrap()
        .clone();
    assert_eq!(item["origin"], "external-link");
    assert_eq!(
        std::fs::read(&db).unwrap(),
        bytes_before,
        "读路径不得迁移或写库"
    );
    assert!(
        stored_skills_policy(&ctx)["repositories"]
            .get("agent-skills")
            .is_some(),
        "读路径不改库"
    );

    unrelated_write(&ctx);
    let skills = stored_skills_policy(&ctx);
    assert!(
        skills["repositories"].get("agent-skills").is_none(),
        "{skills}"
    );
    assert!(
        skills.get("agent-skills").is_none(),
        "集合开关一并移除: {skills}"
    );
    assert_eq!(retire_log_count(&ctx), 1);
    assert!(mirror.join("verification").is_dir(), "已下载的镜像保留不删");

    // 幂等：再写一次不重复记录
    unrelated_write(&ctx);
    assert_eq!(retire_log_count(&ctx), 1);
    let _ = std::fs::remove_dir_all(root);
}

/// 旧库里内置集合已在助手生效（有 bindings）或被用户改过：保持为用户集合，写路径不动。
#[test]
fn legacy_builtin_with_bindings_or_edits_stays_user_collection() {
    let (root, ctx) = temp_ctx("skills-legacy-keep");
    seed_legacy_builtin(
        &ctx,
        serde_json::from_str(LEGACY_BUILTIN_REPO).unwrap(),
        &[("agent-skills", "verification", "codex")],
    );
    let before = stored_skills_policy(&ctx);

    let status = execute(&ctx, &request("skills:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    assert!(status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|el| el["id"] == "skill:agent-skills"));
    assert_eq!(status.data["recommendations"][0]["added"], true);
    unrelated_write(&ctx);
    assert_eq!(stored_skills_policy(&ctx), before);
    assert_eq!(retire_log_count(&ctx), 0);

    // 改过的定义（哪怕无 bindings）也是用户集合
    let (root2, ctx2) = temp_ctx("skills-legacy-edited");
    let mut edited: serde_json::Value = serde_json::from_str(LEGACY_BUILTIN_REPO).unwrap();
    edited["agents"] = serde_json::json!(["codex"]);
    seed_legacy_builtin(&ctx2, edited, &[]);
    let before2 = stored_skills_policy(&ctx2);
    let status2 = execute(&ctx2, &request("skills:status", &[]));
    assert!(status2.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|el| el["id"] == "skill:agent-skills"));
    unrelated_write(&ctx2);
    assert_eq!(stored_skills_policy(&ctx2), before2);
    assert_eq!(retire_log_count(&ctx2), 0);
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(root2);
}

/// 添加推荐来源 = 按推荐里的 definition 写一条普通集合定义（界面走 policy:set）：
/// 集合出现、不限助手、不链接任何助手；逐个启用后只在所选助手出现正确链接。
#[test]
fn adding_recommendation_creates_unassigned_collection_then_per_skill_enable() {
    let (root, ctx) = temp_ctx("skills-add-recommendation");
    let status = execute(&ctx, &request("skills:status", &[]));
    let rec = status.data["recommendations"][0].clone();
    let mut definition = rec["definition"].clone();
    definition["_delete"] = serde_json::json!(false);
    definition["_replace"] = serde_json::json!(true);
    let key = format!("skills.repositories.{}", rec["name"].as_str().unwrap());
    let set = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                &key,
                "--value",
                &definition.to_string(),
            ],
        ),
    );
    assert!(set.ok, "{:?}", set.errors);

    let claude_skills = ctx.home_dir.join(".claude").join("skills");
    let codex_skills = ctx.home_dir.join(".codex").join("skills");
    let status = execute(&ctx, &request("skills:status", &[]));
    let coll = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|el| el["id"] == "skill:agent-skills")
        .expect("添加后集合出现")
        .clone();
    assert_eq!(coll["spec"]["agents"], serde_json::Value::Null);
    assert_eq!(status.data["recommendations"][0]["added"], true);
    assert!(
        !claude_skills.exists() && !codex_skills.exists(),
        "添加不链接任何助手"
    );

    // 仓库取回后（测试不走网络，直接在推荐的 skill_paths 目录下放技能）逐个技能启用
    let mirror = ctx.data_dir.join("repositories").join("agent-skills");
    for dir in rec["skill_paths"].as_array().unwrap() {
        for skill in ["verification", "change-discipline"] {
            let path = mirror.join(dir.as_str().unwrap()).join(skill);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("SKILL.md"), format!("# {skill}")).unwrap();
        }
    }
    let status = execute(&ctx, &request("skills:status", &[]));
    let skill = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|el| el["id"] == "skill:agent-skills/verification")
        .unwrap()
        .clone();
    for agent in ["claude-code", "codex"] {
        let cell = &skill["observed"]["agents"][agent];
        assert_eq!(cell["state"], "not-assigned", "{agent}: {cell}");
        assert!(cell["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "add"));
    }
    let add = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:agent-skills/verification",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(add.ok, "{:?}", add.errors);
    let link = codex_skills.join("verification");
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        std::fs::read_to_string(link.join("SKILL.md")).unwrap(),
        "# verification"
    );
    assert!(
        !claude_skills.join("verification").exists(),
        "未启用的助手不得出现链接"
    );
    assert!(
        !codex_skills.join("change-discipline").exists(),
        "未启用的技能不得出现链接"
    );
    let status = execute(&ctx, &request("skills:status", &[]));
    let cells = &status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|el| el["id"] == "skill:agent-skills/verification")
        .unwrap()["observed"]["agents"];
    assert_eq!(cells["codex"]["state"], "linked");
    assert_eq!(cells["claude-code"]["state"], "not-assigned");
    let _ = std::fs::remove_dir_all(root);
}

/// 实际安装视图：真实目录 / 外部链接 / 本工具链接三类正确分类，查询零写。
#[test]
fn installed_view_classifies_real_entries_with_zero_writes() {
    let (root, ctx) = temp_ctx("skills-installed-view");
    let repo_dir = ctx.data_dir.join("repositories").join("test-skills");
    std::fs::create_dir_all(repo_dir.join("alpha")).unwrap();
    std::fs::write(repo_dir.join("alpha").join("SKILL.md"), "# Alpha").unwrap();
    let set = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.test-skills",
                "--value",
                r#"{"url":"https://example.invalid/test-skills.git"}"#,
            ],
        ),
    );
    assert!(set.ok, "{:?}", set.errors);
    let add = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:test-skills/alpha",
                "--action",
                "add",
                "--agent",
                "claude-code",
            ],
        ),
    );
    assert!(add.ok, "{:?}", add.errors);
    let claude_skills = ctx.home_dir.join(".claude").join("skills");
    std::fs::create_dir_all(claude_skills.join("synced")).unwrap();
    std::fs::write(claude_skills.join("synced").join("SKILL.md"), "# s").unwrap();
    let vendor = ctx.home_dir.join(".vendor").join("skills");
    std::fs::create_dir_all(vendor.join("beta")).unwrap();
    link_dir(vendor.join("beta"), claude_skills.join("beta")).unwrap();

    let db = store::db_path(&ctx);
    let db_before = std::fs::read(&db).unwrap();
    let mtime_before = std::fs::metadata(&db).unwrap().modified().unwrap();
    let list = |dir: &std::path::Path| {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    };
    let dir_before = list(&claude_skills);

    let status = execute(&ctx, &request("skills:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    let installed = status.data["installed"].as_array().unwrap();
    let find = |name: &str| {
        installed
            .iter()
            .find(|item| item["name"] == name && item["agent"] == "claude-code")
            .unwrap_or_else(|| panic!("缺少 {name}: {installed:?}"))
    };
    assert_eq!(find("alpha")["origin"], "managed");
    assert_eq!(find("alpha")["collection"], "test-skills");
    assert_eq!(find("beta")["origin"], "external-link");
    assert_eq!(find("beta")["target"], "~/.vendor/skills/beta");
    assert_eq!(find("beta")["collection"], serde_json::Value::Null);
    assert_eq!(find("synced")["origin"], "native-dir");
    assert_eq!(find("synced")["link_path"], "~/.claude/skills/synced");
    assert_eq!(
        installed.len(),
        3,
        "codex 目录不存在时没有条目: {installed:?}"
    );
    assert_status_matches_contract(&status.data);

    assert_eq!(std::fs::read(&db).unwrap(), db_before, "查询不得写库");
    assert_eq!(
        std::fs::metadata(&db).unwrap().modified().unwrap(),
        mtime_before
    );
    assert_eq!(list(&claude_skills), dir_before, "查询不得改动助手目录");
    assert!(
        !ctx.home_dir.join(".codex").exists(),
        "查询不得创建助手目录"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// status.schema.json 里 status 数据与两个技能字段都是 additionalProperties=false：
/// 输出的键必须与契约声明一致，origin 取值必须在枚举内。
fn assert_status_matches_contract(data: &serde_json::Value) {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../contract/engine-v1/status.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let keys = |value: &serde_json::Value| -> std::collections::BTreeSet<String> {
        value.as_object().unwrap().keys().cloned().collect()
    };
    let declared = keys(&schema["properties"]);
    assert!(
        keys(data).is_subset(&declared),
        "{:?} ⊄ {:?}",
        keys(data),
        declared
    );
    for (field, def) in [
        ("recommendations", "skill_recommendation"),
        ("installed", "skill_installed_entry"),
    ] {
        let def = &schema["$defs"][def];
        let props = keys(&def["properties"]);
        let required: std::collections::BTreeSet<String> = def["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        for item in data[field].as_array().unwrap() {
            let item_keys = keys(item);
            assert!(item_keys.is_subset(&props), "{field}: {item_keys:?}");
            assert!(required.is_subset(&item_keys), "{field}: {item_keys:?}");
        }
    }
    let origins = schema["$defs"]["skill_installed_entry"]["properties"]["origin"]["enum"]
        .as_array()
        .unwrap()
        .clone();
    for item in data["installed"].as_array().unwrap() {
        assert!(origins.contains(&item["origin"]), "{}", item["origin"]);
    }
}

/// 编辑集合要列出来源里的全部技能：不受 skill_paths 限制，带相对路径，不把规则目录、隐藏目录当技能。
#[test]
fn collection_lists_all_available_skills_regardless_of_skill_paths() {
    let (root, context) = temp_ctx("skills-available");
    let repo_dir = context.data_dir.join("repositories").join("picked");
    for rel in [
        "skills/alpha",
        "skills/beta",
        "tools/gamma",
        ".github/hidden",
    ] {
        std::fs::create_dir_all(repo_dir.join(rel)).unwrap();
        std::fs::write(repo_dir.join(rel).join("SKILL.md"), "# x\n").unwrap();
    }
    std::fs::create_dir_all(repo_dir.join("rules")).unwrap();
    std::fs::write(repo_dir.join("rules").join("core.md"), "# core\n").unwrap();
    set_collection(
        &context,
        "picked",
        r#"{"url":"https://example.com/picked.git","skill_paths":["alpha"]}"#,
    );

    let (discovered, error) = collection_discovery(&context, "picked");
    assert!(error.is_null(), "{error}");
    assert_eq!(discovered, ["alpha"], "在用的仍只按 skill_paths");

    let status = execute(&context, &request("skills:status", &[]));
    let coll = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|el| el["id"] == "skill:picked")
        .unwrap()
        .clone();
    assert_eq!(
        coll["observed"]["available"],
        serde_json::json!([
            {"name": "alpha", "path": "skills/alpha"},
            {"name": "beta", "path": "skills/beta"},
            {"name": "gamma", "path": "tools/gamma"},
        ])
    );

    let _ = std::fs::remove_dir_all(root);
}

/// 移除来源并收尾（--purge）：各账号链接、分配记录、下载的仓库镜像与没人再用的快照都清掉；
/// 其他集合仍在用的快照、用户自己的技能目录不动；跟随这个来源的规则不再跟随，规则文件不动。
#[test]
fn removing_a_collection_with_purge_cleans_up_only_what_it_owned() {
    let (root, context) = temp_ctx("skills-purge");
    let make_repo = |name: &str, skill: &str, body: &str| {
        let repo = context.data_dir.join("repositories").join(name);
        std::fs::create_dir_all(repo.join("skills").join(skill)).unwrap();
        std::fs::write(repo.join("skills").join(skill).join("SKILL.md"), body).unwrap();
        std::fs::write(repo.join(".agent-manager-owned"), "{}").unwrap();
        repo
    };
    let gone_repo = make_repo("gone", "alpha", "# alpha\n");
    make_repo("kept", "beta", "# beta\n");
    set_collection(
        &context,
        "gone",
        r#"{"url":"https://example.com/gone.git","skill_paths":["skills"]}"#,
    );
    set_collection(
        &context,
        "kept",
        r#"{"url":"https://example.com/kept.git","skill_paths":["skills"]}"#,
    );
    assert!(skill_action(&context, "skill:gone/alpha", "add", "claude-code").ok);
    assert!(skill_action(&context, "skill:kept/beta", "add", "claude-code").ok);
    // 用户自己的技能目录
    let own = context.home_dir.join(".claude/skills/mine");
    std::fs::create_dir_all(&own).unwrap();
    std::fs::write(own.join("SKILL.md"), "# mine\n").unwrap();
    // 规则文件跟随 gone/core 与 kept/core 各一条
    {
        let conn = rusqlite::Connection::open(context.data_dir.join("native-manager.db")).unwrap();
        conn.execute_batch("CREATE TABLE IF NOT EXISTS rule_source_applied (target TEXT PRIMARY KEY NOT NULL, rule TEXT NOT NULL, digest TEXT NOT NULL, applied_at TEXT NOT NULL)").unwrap();
        conn.execute("INSERT INTO rule_source_applied VALUES ('rules:claude_global','gone/core','d','t'), ('rules:codex_global','kept/core','d','t')", []).unwrap();
    }
    let versions = |ctx: &Context| {
        std::fs::read_dir(ctx.data_dir.join("skill-versions"))
            .unwrap()
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .count()
    };
    assert_eq!(versions(&context), 2);

    let removed = execute(
        &context,
        &request(
            "skills:apply",
            &["--only", "skill:gone", "--action", "remove", "--purge"],
        ),
    );
    assert!(removed.ok, "{:?}", removed.errors);

    assert!(
        !context.home_dir.join(".claude/skills/alpha").exists(),
        "这个集合的链接移除了"
    );
    assert!(
        context.home_dir.join(".claude/skills/beta").exists(),
        "别的集合的链接不动"
    );
    assert_eq!(
        std::fs::read_to_string(own.join("SKILL.md")).unwrap(),
        "# mine\n",
        "用户自己的技能目录不动"
    );
    assert!(!gone_repo.exists(), "下载的仓库镜像删掉了");
    assert!(context.data_dir.join("repositories/kept").exists());
    assert_eq!(versions(&context), 1, "只留下别的集合还在用的快照");
    assert!(
        binding_agents(&context, "gone", "alpha").is_empty(),
        "分配记录删掉了"
    );
    let conn = rusqlite::Connection::open(context.data_dir.join("native-manager.db")).unwrap();
    let follows: Vec<String> = conn
        .prepare("SELECT rule FROM rule_source_applied ORDER BY rule")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(follows, ["kept/core"], "只解除跟随被移除来源的规则");
    let _ = std::fs::remove_dir_all(root);
}

/// 不带 --purge 的移除：链接移除、规则不再跟随，但下载的仓库与快照都保留（可以再加回来）。
#[test]
fn removing_a_collection_without_purge_keeps_downloads() {
    let (root, context) = temp_ctx("skills-remove-keep");
    let repo = context.data_dir.join("repositories").join("gone");
    std::fs::create_dir_all(repo.join("skills/alpha")).unwrap();
    std::fs::write(repo.join("skills/alpha/SKILL.md"), "# alpha\n").unwrap();
    std::fs::write(repo.join(".agent-manager-owned"), "{}").unwrap();
    set_collection(
        &context,
        "gone",
        r#"{"url":"https://example.com/gone.git","skill_paths":["skills"]}"#,
    );
    assert!(skill_action(&context, "skill:gone/alpha", "add", "claude-code").ok);
    let removed = execute(
        &context,
        &request(
            "skills:apply",
            &["--only", "skill:gone", "--action", "remove"],
        ),
    );
    assert!(removed.ok, "{:?}", removed.errors);
    assert!(!context.home_dir.join(".claude/skills/alpha").exists());
    assert!(repo.exists());
    let _ = std::fs::remove_dir_all(root);
}

/// 「改成我的版本」：复制现在用着的那一版到用户目录（可改），登记为用户的本机集合并记下来历；
/// 账号换过去后上游出新版只提示、不动用户文件；同名目录或指向别处的集合名一律拒绝。
#[test]
fn forking_a_skill_copies_it_into_the_users_collection_and_tracks_upstream() {
    let (root, context) = temp_ctx("skills-fork");
    let repo = context.data_dir.join("repositories").join("upstream");
    std::fs::create_dir_all(repo.join("skills/alpha")).unwrap();
    std::fs::write(repo.join("skills/alpha/SKILL.md"), "# alpha v1\n").unwrap();
    std::fs::write(repo.join(".agent-manager-owned"), "{}").unwrap();
    set_collection(
        &context,
        "upstream",
        r#"{"url":"https://example.com/upstream.git","skill_paths":["skills"]}"#,
    );
    assert!(skill_action(&context, "skill:upstream/alpha", "add", "claude-code").ok);

    let mine_root = root.join("mine");
    let dest = mine_root.to_string_lossy().to_string();
    let forked = execute(
        &context,
        &request(
            "skills:fork",
            &["--only", "skill:upstream/alpha", "--dest", &dest],
        ),
    );
    assert!(forked.ok, "{:?}", forked.errors);
    assert_eq!(forked.data["collection"], "my-skills");
    let copy = mine_root.join("alpha/SKILL.md");
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "# alpha v1\n");
    std::fs::write(&copy, "# alpha v1\n我的修改\n").expect("复制出来的文件可以改");

    // 账号换到用户的版本：先取消原分配，再加用户的
    assert!(skill_action(&context, "skill:upstream/alpha", "remove", "claude-code").ok);
    let added = skill_action(&context, "skill:my-skills/alpha", "add", "claude-code");
    assert!(added.ok, "{:?}", added.errors);
    let link = context.home_dir.join(".claude/skills/alpha");
    assert_eq!(
        std::fs::read_to_string(link.join("SKILL.md")).unwrap(),
        "# alpha v1\n我的修改\n"
    );

    let status = execute(&context, &request("skills:status", &[]));
    let element = |id: &str| {
        status.data["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(
        element("skill:my-skills/alpha")["observed"]["forked_from"]["collection"],
        "upstream"
    );
    assert_eq!(
        element("skill:my-skills/alpha")["observed"]["forked_from"]["upstream_changed"],
        false
    );

    // 上游出新版：只提示，用户文件不动
    std::fs::write(repo.join("skills/alpha/SKILL.md"), "# alpha v2\n").unwrap();
    let status = execute(&context, &request("skills:status", &[]));
    let forked_from = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "skill:my-skills/alpha")
        .unwrap()["observed"]["forked_from"]
        .clone();
    assert_eq!(forked_from["upstream_changed"], true);
    assert_eq!(
        std::fs::read_to_string(&copy).unwrap(),
        "# alpha v1\n我的修改\n"
    );

    // 再复制一次：目录已经有了，不覆盖
    let again = execute(
        &context,
        &request(
            "skills:fork",
            &["--only", "skill:upstream/alpha", "--dest", &dest],
        ),
    );
    assert!(!again.ok);
    assert!(format!("{:?}", again.errors).contains("不覆盖"));
    // 集合名已指向别处：拒绝
    let elsewhere = root.join("other").to_string_lossy().to_string();
    let clash = execute(
        &context,
        &request(
            "skills:fork",
            &["--only", "skill:upstream/alpha", "--dest", &elsewhere],
        ),
    );
    assert!(!clash.ok);
    assert!(format!("{:?}", clash.errors).contains("已经指向别处"));
    let _ = std::fs::remove_dir_all(root);
}

/// 回归：换成用户的版本后，停用或移除原集合只动它自己的链接，不碰已归用户集合的同名链接。
#[test]
fn disabling_or_removing_the_origin_keeps_the_users_version_linked() {
    let (root, context) = temp_ctx("skills-fork-origin-removed");
    let repo = context.data_dir.join("repositories").join("upstream");
    std::fs::create_dir_all(repo.join("skills/alpha")).unwrap();
    std::fs::write(repo.join("skills/alpha/SKILL.md"), "# alpha\n").unwrap();
    std::fs::write(repo.join(".agent-manager-owned"), "{}").unwrap();
    set_collection(
        &context,
        "upstream",
        r#"{"url":"https://example.com/upstream.git","skill_paths":["skills"]}"#,
    );
    assert!(skill_action(&context, "skill:upstream/alpha", "add", "claude-code").ok);
    let dest = root.join("mine").to_string_lossy().to_string();
    assert!(
        execute(
            &context,
            &request(
                "skills:fork",
                &["--only", "skill:upstream/alpha", "--dest", &dest]
            )
        )
        .ok
    );
    assert!(skill_action(&context, "skill:upstream/alpha", "remove", "claude-code").ok);
    assert!(skill_action(&context, "skill:my-skills/alpha", "add", "claude-code").ok);
    let link = context.home_dir.join(".claude/skills/alpha");
    let mine = root.join("mine/alpha");

    let disabled = execute(
        &context,
        &request(
            "skills:apply",
            &["--only", "skill:upstream", "--action", "disable"],
        ),
    );
    assert!(disabled.ok, "{:?}", disabled.errors);
    assert_eq!(
        std::fs::canonicalize(&link).unwrap(),
        std::fs::canonicalize(&mine).unwrap(),
        "停用原集合不碰用户版本的链接"
    );

    let removed = execute(
        &context,
        &request(
            "skills:apply",
            &["--only", "skill:upstream", "--action", "remove", "--purge"],
        ),
    );
    assert!(removed.ok, "{:?}", removed.errors);
    assert_eq!(
        std::fs::canonicalize(&link).unwrap(),
        std::fs::canonicalize(&mine).unwrap(),
        "移除原集合不碰用户版本的链接"
    );
    assert!(mine.join("SKILL.md").is_file());
    let _ = std::fs::remove_dir_all(root);
}

/// 其他来源（unmanaged）：复刻 2026-09-29 真实机器上的布局——`npx skills` 锁文件、带
/// `skill-release.json` 的真实目录、Claude `synced/`、Codex 共用目录 `~/.agents/skills`
/// 里与受管技能同名的链接、指向 git 仓库内的链接。识别来源、重名与能否纳入；全程零写。
#[test]
fn unmanaged_view_detects_sources_shared_dir_and_duplicates_with_zero_writes() {
    let (root, ctx) = temp_ctx("skills-unmanaged-view");
    let repo_dir = ctx.data_dir.join("repositories").join("test-skills");
    std::fs::create_dir_all(repo_dir.join("alpha")).unwrap();
    std::fs::write(repo_dir.join("alpha").join("SKILL.md"), "# Alpha").unwrap();
    let set = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.test-skills",
                "--value",
                r#"{"url":"https://example.invalid/test-skills.git"}"#,
            ],
        ),
    );
    assert!(set.ok, "{:?}", set.errors);
    for agent in ["claude-code", "codex"] {
        let add = execute(
            &ctx,
            &request(
                "skills:apply",
                &[
                    "--only",
                    "skill:test-skills/alpha",
                    "--action",
                    "add",
                    "--agent",
                    agent,
                ],
            ),
        );
        assert!(add.ok, "{agent}: {:?}", add.errors);
    }
    let home = &ctx.home_dir;
    let claude_skills = home.join(".claude/skills");
    let shared = home.join(".agents/skills");
    // npx skills 装的 archify：Claude 与共用目录各一份真实目录，带发布信息
    for dir in [claude_skills.join("archify"), shared.join("archify")] {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "# archify").unwrap();
        std::fs::write(
            dir.join("skill-release.json"),
            r#"{"version":"3.0.1","source":{"repository":"https://github.com/tt-a1i/archify"}}"#,
        )
        .unwrap();
    }
    std::fs::create_dir_all(shared.join("orca-cli")).unwrap();
    std::fs::write(shared.join("orca-cli/SKILL.md"), "# orca").unwrap();
    std::fs::write(
        home.join(".agents/.skill-lock.json"),
        r#"{"version":3,"skills":{
            "archify":{"sourceUrl":"https://github.com/tt-a1i/archify.git","skillPath":"archify/SKILL.md"},
            "orca-cli":{"sourceUrl":"https://github.com/stablyai/orca.git","skillPath":"skills/orca-cli/SKILL.md"}}}"#,
    )
    .unwrap();
    // Claude 自己同步的目录：没有 SKILL.md，不能纳入
    std::fs::create_dir_all(claude_skills.join("synced/bucket-1")).unwrap();
    // 共用目录里指向 git 仓库内技能的链接
    let git_repo = home.join("src/team-skills");
    std::fs::create_dir_all(git_repo.join(".git")).unwrap();
    std::fs::write(
        git_repo.join(".git/config"),
        "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = git@example.invalid:team/skills.git\n",
    )
    .unwrap();
    std::fs::create_dir_all(git_repo.join("skills/gitty")).unwrap();
    std::fs::write(git_repo.join("skills/gitty/SKILL.md"), "# gitty").unwrap();
    link_dir(git_repo.join("skills/gitty"), shared.join("gitty")).unwrap();
    // 与 Codex 上受管的 alpha 同名：Codex 会加载两份
    link_dir(repo_dir.join("alpha"), shared.join("alpha")).unwrap();

    let db = store::db_path(&ctx);
    let db_before = std::fs::read(&db).unwrap();
    let tree = |dir: &std::path::Path| {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    };
    let shared_before = tree(&shared);
    let claude_before = tree(&claude_skills);
    let lock_before = std::fs::read(home.join(".agents/.skill-lock.json")).unwrap();

    let status = execute(&ctx, &request("skills:status", &[]));
    assert!(status.ok, "{:?}", status.errors);
    assert_status_matches_contract(&status.data);
    let unmanaged = &status.data["unmanaged"];
    let skills = unmanaged["skills"].as_array().unwrap();
    let names: Vec<&str> = skills.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        ["archify", "gitty", "orca-cli", "synced"],
        "受管的 alpha 不算其他来源"
    );
    let find = |name: &str| skills.iter().find(|s| s["name"] == name).unwrap();

    let archify = find("archify");
    let places: Vec<(serde_json::Value, serde_json::Value, &str)> = archify["locations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["agent"].clone(),
                l["shared"].clone(),
                l["origin"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        places,
        [
            (
                serde_json::json!("claude-code"),
                serde_json::Value::Null,
                "native-dir"
            ),
            (
                serde_json::Value::Null,
                serde_json::json!("codex-shared"),
                "native-dir"
            ),
        ]
    );
    assert_eq!(archify["adoptable"], true);
    assert_eq!(
        archify["source"]["repository"],
        "https://github.com/tt-a1i/archify.git"
    );
    assert_eq!(archify["source"]["path"], "archify");
    assert_eq!(archify["source"]["version"], "3.0.1");
    assert_eq!(archify["source"]["installer"], "npx-skills");
    assert_eq!(
        archify["source"]["evidence"],
        serde_json::json!(["skill-lock", "skill-release"])
    );

    assert_eq!(find("orca-cli")["source"]["path"], "skills/orca-cli");
    assert_eq!(
        find("orca-cli")["source"]["version"],
        serde_json::Value::Null
    );

    let gitty = find("gitty");
    assert_eq!(gitty["locations"][0]["origin"], "external-link");
    assert_eq!(
        gitty["locations"][0]["target"],
        "~/src/team-skills/skills/gitty"
    );
    assert_eq!(
        gitty["source"]["repository"],
        "git@example.invalid:team/skills.git"
    );
    assert_eq!(gitty["source"]["path"], "skills/gitty");
    assert_eq!(gitty["source"]["evidence"], serde_json::json!(["git"]));

    let synced = find("synced");
    assert_eq!(synced["adoptable"], false);
    assert_eq!(synced["blocked_reason"], "claude-synced");
    assert_eq!(synced["source"], serde_json::Value::Null);

    assert_eq!(
        unmanaged["shared_dirs"],
        serde_json::json!([{"id": "codex-shared", "kind": "codex", "path": "~/.agents/skills", "exists": true}])
    );
    assert_eq!(
        unmanaged["duplicates"],
        serde_json::json!([{"name": "alpha", "agent": "codex", "paths": ["~/.codex/skills/alpha", "~/.agents/skills/alpha"]}])
    );

    assert_eq!(std::fs::read(&db).unwrap(), db_before, "查询不得写库");
    assert_eq!(tree(&shared), shared_before, "查询不得改动共用目录");
    assert_eq!(tree(&claude_skills), claude_before, "查询不得改动助手目录");
    assert_eq!(
        std::fs::read(home.join(".agents/.skill-lock.json")).unwrap(),
        lock_before,
        "不得改锁文件"
    );
    let _ = std::fs::remove_dir_all(root);
}

// ---------- git 来源：版本跟踪、只下载指定目录、切换与回滚 ----------

/// 真实 git 仓库夹具：v1.0.0 → v1.1.0 → main 上再一个提交（打 v2.0.0-rc.1 预发布标签）。
/// skills/alpha 是要用的技能；skills/beta 与 big/ 不在 skill_paths 里，不应被下载。
#[test]
fn git_source_tracks_versions_downloads_only_listed_dirs_and_switches() {
    let (root, ctx) = temp_ctx("skills-versioning");
    let git = |dir: &std::path::Path, args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .expect("git 可执行");
        assert!(
            out.status.success(),
            "git {args:?} 失败: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let work = root.join("work");
    let write = |rel: &str, text: &str| {
        let path = work.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    std::fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    write("skills/alpha/SKILL.md", "# alpha v1.0");
    write("skills/beta/SKILL.md", "# beta");
    write("big/blob.txt", "not a skill");
    write("rules/team.md", "# team rule");
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-qm", "v1.0"]);
    git(&work, &["tag", "v1.0.0"]);
    let c1 = git(&work, &["rev-parse", "HEAD"]);
    write("skills/alpha/SKILL.md", "# alpha v1.1");
    git(&work, &["commit", "-qam", "v1.1"]);
    git(&work, &["tag", "-a", "v1.1.0", "-m", "release 1.1"]);
    let c2 = git(&work, &["rev-parse", "HEAD"]);
    write("skills/alpha/SKILL.md", "# alpha next");
    git(&work, &["commit", "-qam", "next"]);
    git(&work, &["tag", "v2.0.0-rc.1"]);
    let c3 = git(&work, &["rev-parse", "HEAD"]);
    // 裸仓库做远端，并允许部分克隆（file:// 才会走过滤）
    let bare = root.join("origin.git");
    git(
        &root,
        &[
            "clone",
            "-q",
            "--bare",
            &work.to_string_lossy(),
            &bare.to_string_lossy(),
        ],
    );
    git(&bare, &["config", "uploadpack.allowFilter", "true"]);
    let url = format!("file://{}", bare.to_string_lossy());

    let define = |extra: &str| {
        let set = execute(
            &ctx,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "skills.repositories.src",
                    "--value",
                    &format!(
                        r#"{{"url":{},"skill_paths":["skills/alpha"]{extra}}}"#,
                        serde_json::to_string(&url).unwrap()
                    ),
                ],
            ),
        );
        assert!(set.ok, "{:?}", set.errors);
    };
    define("");
    let update = |args: &[&str]| {
        let mut all = vec!["--only", "skill:src", "--action", "update"];
        all.extend_from_slice(args);
        execute(&ctx, &request("skills:apply", &all))
    };
    let repo = ctx.data_dir.join("repositories").join("src");
    let head = || git(&repo, &["rev-parse", "HEAD"]);
    let collection = || {
        let status = execute(&ctx, &request("skills:status", &[]));
        assert!(status.ok, "{:?}", status.errors);
        status.data["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == "skill:src")
            .cloned()
            .unwrap()
    };

    // 1. 自动：有版本号标签 → 最新正式版 v1.1.0（预发布不算）；只下载 skills/alpha 与 rules
    let env = update(&[]);
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(head(), c2);
    assert_eq!(
        std::fs::read_to_string(repo.join("skills/alpha/SKILL.md")).unwrap(),
        "# alpha v1.1"
    );
    assert!(repo.join("rules/team.md").is_file(), "规则目录要一起取回");
    assert!(!repo.join("skills/beta").exists(), "没列出的技能目录不下载");
    assert!(!repo.join("big").exists(), "其他目录不下载");
    assert_eq!(
        git(
            &repo,
            &["config", "--get", "remote.origin.partialclonefilter"]
        ),
        "tree:0"
    );
    let coll = collection();
    assert_eq!(coll["observed"]["version"]["current"]["label"], "v1.1.0");
    assert_eq!(coll["observed"]["version"]["current"]["kind"], "tag");
    let available: Vec<&str> = coll["observed"]["available"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        available,
        ["alpha", "beta"],
        "没下载的技能也能在编辑来源时勾选"
    );

    // 2. 版本清单：只读网络，不写库、不动镜像
    let db_before = std::fs::read(store::db_path(&ctx)).unwrap();
    let versions = execute(&ctx, &request("skills:versions", &["--only", "skill:src"]));
    assert!(versions.ok, "{:?}", versions.errors);
    let tags: Vec<&str> = versions.data["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(tags, ["v2.0.0-rc.1", "v1.1.0", "v1.0.0"]);
    assert_eq!(versions.data["tags"][0]["prerelease"], true);
    assert_eq!(versions.data["latest"]["name"], "v1.1.0");
    assert_eq!(versions.data["auto_resolved"], "tag");
    assert_eq!(versions.data["current"]["label"], "v1.1.0");
    assert_eq!(versions.data["default_branch"], "main");
    assert_eq!(std::fs::read(store::db_path(&ctx)).unwrap(), db_before);
    assert_eq!(head(), c2);
    let wrong = execute(
        &ctx,
        &request("skills:versions", &["--only", "skill:src/alpha"]),
    );
    assert!(!wrong.ok);

    // 3. 技能链接到 codex，再回滚到 v1.0.0：镜像回到 v1.0，技能按既有语义换版本
    let add = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:src/alpha",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(add.ok, "{:?}", add.errors);
    let link = ctx.home_dir.join(".codex/skills/alpha/SKILL.md");
    assert_eq!(std::fs::read_to_string(&link).unwrap(), "# alpha v1.1");
    let env = update(&["--ref", "v1.0.0"]);
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(head(), c1);
    assert_eq!(
        collection()["observed"]["version"]["current"]["label"],
        "v1.0.0"
    );
    assert_eq!(
        std::fs::read_to_string(&link).unwrap(),
        "# alpha v1.1",
        "镜像换了版本，账号链接要等技能更新才换"
    );
    let relink = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:src/alpha",
                "--action",
                "update",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(relink.ok, "{:?}", relink.errors);
    assert_eq!(std::fs::read_to_string(&link).unwrap(), "# alpha v1.0");

    // 3b. 预览换到最新：列出 alpha 的文件变化与会跟着换的账号；镜像检出与库都不动
    let db_before = std::fs::read(store::db_path(&ctx)).unwrap();
    let preview = execute(&ctx, &request("skills:preview", &["--only", "skill:src"]));
    assert!(preview.ok, "{:?}", preview.errors);
    assert_eq!(preview.data["from"]["label"], "v1.0.0");
    assert_eq!(preview.data["to"]["name"], "v1.1.0");
    assert_eq!(preview.data["same"], false);
    assert_eq!(
        preview.data["skills"],
        serde_json::json!([{"name": "alpha", "path": "skills/alpha", "files": [{"status": "modified", "file": "SKILL.md", "original": "# alpha v1.0", "content": "# alpha v1.1", "contents": true}], "accounts": ["codex"]}])
    );
    assert_eq!(preview.data["rules"], serde_json::json!([]));
    assert_eq!(head(), c1, "预览不检出");
    assert_eq!(
        std::fs::read(store::db_path(&ctx)).unwrap(),
        db_before,
        "预览不写库"
    );
    let same = execute(
        &ctx,
        &request(
            "skills:preview",
            &["--only", "skill:src", "--ref", "v1.0.0"],
        ),
    );
    assert_eq!(same.data["same"], true);
    let bad = execute(
        &ctx,
        &request("skills:preview", &["--only", "skill:src", "--ref", "-x"]),
    );
    assert!(!bad.ok);

    // 4. 检查更新（真实 git，file:// 远端）：自动跟踪下落后于 v1.1.0
    let probe = execute(
        &ctx,
        &request("system:refresh-upstream", &["--only", "skill:src"]),
    );
    assert!(probe.ok, "{:?}", probe.errors);
    let freshness = collection()["freshness"].clone();
    assert_eq!(freshness["state"], "behind");
    assert_eq!(freshness["current"], "v1.0.0");
    assert_eq!(freshness["upstream"], "v1.1.0");

    // 5. 改跟踪方式：主分支 → main 的最新提交；固定 → 不再跟最新
    define(r#","track":{"mode":"default"}"#);
    assert!(update(&[]).ok);
    assert_eq!(head(), c3);
    assert_eq!(
        collection()["observed"]["version"]["current"]["label"],
        "main"
    );
    define(r#","track":{"mode":"pinned","ref":"v1.0.0"}"#);
    assert!(update(&[]).ok);
    assert_eq!(head(), c1);
    define(r#","track":{"mode":"tag","prerelease":true}"#);
    assert!(update(&[]).ok);
    assert_eq!(head(), c3, "允许预发布时跟到 v2.0.0-rc.1");

    // 6. 拒绝：不合法的版本名、不存在的版本（镜像不动）、写错的跟踪方式
    let bad = update(&["--ref", "--upload-pack=touch /tmp/x"]);
    assert!(!bad.ok);
    let missing = update(&["--ref", "v9.9.9"]);
    assert!(!missing.ok);
    assert_eq!(head(), c3, "解析不出目标时镜像不动");
    define(r#","track":{"mode":"nightly"}"#);
    let broken = update(&[]);
    assert!(!broken.ok);
    assert!(collection()["observed"]["version"]["track_error"]
        .as_str()
        .unwrap()
        .contains("nightly"));
    assert_eq!(head(), c3);

    // 7. 操作记录带上从哪个提交到哪个版本
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let detail: String = conn
        .query_row(
            "SELECT detail_json FROM operation_logs WHERE operation_type = 'skills:update' AND detail_json LIKE '%\"to\"%' ORDER BY rowid DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        detail.contains("v2.0.0-rc.1") && detail.contains(&c3),
        "{detail}"
    );
    let _ = std::fs::remove_dir_all(root);
}

// ---------- 纳入管理：只留一份源，可撤回 ----------

fn tree_digest(dir: &std::path::Path) -> String {
    agent_manager_engine::native_skills::hash_skill_dir(dir).unwrap()
}

#[test]
#[cfg(unix)] // 用到 Unix 权限位
fn adopt_unmanaged_skills_keeps_one_source_and_can_be_undone() {
    let (root, ctx) = temp_ctx("skills-adopt");
    let git = |dir: &std::path::Path, args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .expect("git 可执行");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let write = |path: std::path::PathBuf, text: &str| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    // 远端：skills/archy 有 v1.0.0 与 v1.1.0；skills/other 用来测「本地改过」
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    write(work.join("skills/archy/SKILL.md"), "# archy 1.0");
    write(work.join("skills/other/SKILL.md"), "# other");
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-qm", "1.0"]);
    git(&work, &["tag", "v1.0.0"]);
    write(work.join("skills/archy/SKILL.md"), "# archy 1.1");
    git(&work, &["commit", "-qam", "1.1"]);
    git(&work, &["tag", "v1.1.0"]);
    let bare = root.join("origin.git");
    git(
        &root,
        &[
            "clone",
            "-q",
            "--bare",
            &work.to_string_lossy(),
            &bare.to_string_lossy(),
        ],
    );
    git(&bare, &["config", "uploadpack.allowFilter", "true"]);
    let url = format!("file://{}", bare.to_string_lossy());

    let home = ctx.home_dir.clone();
    let claude = home.join(".claude/skills");
    let codex = home.join(".codex/skills");
    let shared = home.join(".agents/skills");
    // npx skills 装的 archy（旧版本 1.0.0）：Claude 与共用目录各一份
    for dir in [claude.join("archy"), shared.join("archy")] {
        write(dir.join("SKILL.md"), "# archy 1.0");
        write(dir.join("skill-release.json"), r#"{"version":"1.0.0"}"#);
    }
    // 真实目录里多了 skill-release.json，来源里没有：比对时要算不一致 → 这份先去掉，让它与 v1.0.0 一致
    agent_manager_engine::platform::remove_link(claude.join("archy/skill-release.json")).unwrap();
    agent_manager_engine::platform::remove_link(shared.join("archy/skill-release.json")).unwrap();
    write(claude.join("edited/SKILL.md"), "# other (我改过)");
    write(claude.join("mine/SKILL.md"), "# 我自己写的");
    std::fs::create_dir_all(claude.join("synced/bucket")).unwrap();
    write(
        home.join(".agents/.skill-lock.json"),
        &format!(
            r#"{{"version":3,"skills":{{
                "archy":{{"sourceUrl":"{url}","skillPath":"skills/archy/SKILL.md"}},
                "edited":{{"sourceUrl":"{url}","skillPath":"skills/other/SKILL.md"}}}}}}"#
        ),
    );
    // 已管理的 dup：本机来源链接到 codex，共用目录里还有一份链接
    let local_src = home.join("tools/skills");
    write(local_src.join("dup/SKILL.md"), "# dup");
    let set = execute(
        &ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                "skills.repositories.loc",
                "--value",
                &format!(r#"{{"source_path":"{}"}}"#, local_src.to_string_lossy()),
            ],
        ),
    );
    assert!(set.ok, "{:?}", set.errors);
    let add = execute(
        &ctx,
        &request(
            "skills:apply",
            &[
                "--only",
                "skill:loc/dup",
                "--action",
                "add",
                "--agent",
                "codex",
            ],
        ),
    );
    assert!(add.ok, "{:?}", add.errors);
    link_dir(local_src.join("dup"), shared.join("dup")).unwrap();
    let lock_before = std::fs::read(home.join(".agents/.skill-lock.json")).unwrap();
    let claude_archy_digest = tree_digest(&claude.join("archy"));

    let preview = |name: &str| execute(&ctx, &request("skills:adopt-preview", &["--name", name]));
    let adopt = |args: &[&str]| execute(&ctx, &request("skills:adopt", args));

    // 1. 预览 archy：本地是 v1.0.0 → 按 v1.0.0 纳入；全程零写
    let db_before = std::fs::read(store::db_path(&ctx)).unwrap();
    let p = preview("archy");
    assert!(p.ok, "{:?}", p.errors);
    let check = &p.data["source_check"];
    assert_eq!(check["same"], true, "{check}");
    assert_eq!(
        p.data["current_accounts"],
        serde_json::json!(["claude-code", "codex"]),
        "Claude 目录 + 共用目录（对应全部 Codex 账号）"
    );
    assert_eq!(check["adopt_at"]["name"], "v1.0.0");
    assert_eq!(check["latest"]["name"], "v1.1.0");
    assert_eq!(check["ref"], "v1.0.0");
    assert_eq!(
        check["accounts"],
        serde_json::json!(["claude-code", "codex"])
    );
    assert_eq!(p.data["collection"]["name"], "origin");
    assert_eq!(
        p.data["removes"],
        serde_json::json!(["~/.claude/skills/archy", "~/.agents/skills/archy"])
    );
    assert_eq!(
        std::fs::read(store::db_path(&ctx)).unwrap(),
        db_before,
        "预览不写库"
    );
    assert!(
        claude.join("archy/SKILL.md").is_file() && shared.join("archy").is_dir(),
        "预览不动目录"
    );
    assert!(
        !ctx.data_dir.join("repositories/origin").exists(),
        "预览不在数据目录里下载"
    );

    // 2. 过期指纹被拒，什么都不动
    let stale = adopt(&[
        "--name",
        "archy",
        "--digest",
        "0000",
        "--ref",
        "v1.0.0",
        "--path",
        "skills/archy",
    ]);
    assert!(!stale.ok);
    assert!(claude.join("archy").is_dir() && !claude.join("archy").is_symlink());

    // 3. 纳入 archy：只留一份源，两个账号都链接上，共用目录那份清掉
    let digest = check["digest"].as_str().unwrap();
    let done = adopt(&[
        "--name",
        "archy",
        "--digest",
        digest,
        "--ref",
        "v1.0.0",
        "--path",
        "skills/archy",
    ]);
    assert!(done.ok, "{:?}", done.errors);
    let backup = done.data["backup"].as_str().unwrap().to_string();
    assert!(claude.join("archy").is_symlink() && codex.join("archy").is_symlink());
    assert_eq!(
        std::fs::read_to_string(codex.join("archy/SKILL.md")).unwrap(),
        "# archy 1.0"
    );
    assert!(
        std::fs::symlink_metadata(shared.join("archy")).is_err(),
        "共用目录的副本要清掉"
    );
    let status = execute(&ctx, &request("skills:status", &[]));
    let coll = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "skill:origin")
        .unwrap()
        .clone();
    assert_eq!(
        coll["spec"]["skill_paths"],
        serde_json::json!(["skills/archy"])
    );
    assert_eq!(coll["observed"]["version"]["current"]["label"], "v1.0.0");
    assert!(status.data["unmanaged"]["skills"]
        .as_array()
        .unwrap()
        .iter()
        .all(|s| s["name"] != "archy"));
    assert_eq!(
        std::fs::read(home.join(".agents/.skill-lock.json")).unwrap(),
        lock_before,
        "不改 npx skills 的锁文件"
    );

    // 4. 撤回：链接与登记撤掉，原目录原样放回；不能撤两次
    let undo = adopt(&["--undo", &backup]);
    assert!(undo.ok, "{:?}", undo.errors);
    assert!(claude.join("archy").is_dir() && !claude.join("archy").is_symlink());
    assert_eq!(tree_digest(&claude.join("archy")), claude_archy_digest);
    assert!(shared.join("archy/SKILL.md").is_file());
    assert!(std::fs::symlink_metadata(codex.join("archy")).is_err());
    let status = execute(&ctx, &request("skills:status", &[]));
    assert!(status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["id"] != "skill:origin"));
    assert!(!adopt(&["--undo", &backup]).ok, "不能撤两次");

    // 5. 本地改过：不选「用来源的版本」就拒绝；选了才覆盖（原内容在备份里）
    let p = preview("edited");
    let check = &p.data["source_check"];
    assert_eq!(check["same"], false);
    assert_eq!(
        check["locations"][0]["files"],
        serde_json::json!([{"status": "modified", "file": "SKILL.md", "original": "# other (我改过)", "content": "# other", "contents": true}])
    );
    assert_eq!(
        p.data["collection"]["linked_as"], "other",
        "本地换过名字：按来源里的名字链接"
    );
    let digest = check["digest"].as_str().unwrap();
    let no = adopt(&[
        "--name",
        "edited",
        "--digest",
        digest,
        "--path",
        "skills/other",
    ]);
    assert!(!no.ok);
    assert!(claude.join("edited").is_dir() && !claude.join("edited").is_symlink());
    let yes = adopt(&[
        "--name",
        "edited",
        "--digest",
        digest,
        "--path",
        "skills/other",
        "--accept-source",
    ]);
    assert!(yes.ok, "{:?}", yes.errors);
    assert_eq!(
        std::fs::read_to_string(claude.join("other/SKILL.md")).unwrap(),
        "# other"
    );
    assert!(
        std::fs::symlink_metadata(claude.join("edited")).is_err(),
        "原名字的目录已移进备份"
    );

    // 6. 已管理的 dup：只清掉共用目录那份，codex 上的受管链接不动；可撤回
    let p = preview("dup");
    assert_eq!(p.data["kind"], "dedupe");
    let d = adopt(&[
        "--name",
        "dup",
        "--digest",
        p.data["digest"].as_str().unwrap(),
    ]);
    assert!(d.ok, "{:?}", d.errors);
    assert!(std::fs::symlink_metadata(shared.join("dup")).is_err());
    assert!(codex.join("dup").is_symlink());
    assert!(adopt(&["--undo", d.data["backup"].as_str().unwrap()]).ok);
    assert_eq!(
        std::fs::read_link(shared.join("dup")).unwrap(),
        local_src.join("dup")
    );

    // 7. 认不出来源的 mine：保留本地内容，放进 ~/my-skills 作为本机来源；可撤回
    let p = preview("mine");
    assert!(p.data["source_check"]["error"].is_string());
    let local = &p.data["local"];
    assert_eq!(local["dest"], "~/my-skills/mine");
    // 原来只在 Claude 上：预览标出原来就有的账号；只勾 Claude 时 Codex 不链接
    assert_eq!(
        p.data["current_accounts"],
        serde_json::json!(["claude-code"])
    );
    let digest = local["digest"].as_str().unwrap().to_string();
    let bad_agent = adopt(&[
        "--name",
        "mine",
        "--digest",
        &digest,
        "--keep-local",
        "--agents",
        "nope",
    ]);
    assert!(!bad_agent.ok, "不存在的账号拒绝");
    assert!(
        claude.join("mine").is_dir() && !claude.join("mine").is_symlink(),
        "拒绝后原样恢复"
    );
    assert!(
        !home.join("my-skills/mine").exists(),
        "拒绝后复制出的目录也撤掉"
    );
    let m = adopt(&[
        "--name",
        "mine",
        "--digest",
        &digest,
        "--keep-local",
        "--agents",
        "claude-code",
    ]);
    assert!(m.ok, "{:?}", m.errors);
    assert_eq!(m.data["linked"], serde_json::json!(["claude-code"]));
    assert_eq!(
        std::fs::read_to_string(home.join("my-skills/mine/SKILL.md")).unwrap(),
        "# 我自己写的"
    );
    assert!(
        claude.join("mine").is_symlink() && std::fs::symlink_metadata(codex.join("mine")).is_err()
    );
    let progress = execute(&ctx, &request("runs:progress", &["--run", "test-run-1"]));
    assert_eq!(
        progress.data["progress"],
        serde_json::Value::Null,
        "操作结束后进度文件删掉"
    );
    assert!(adopt(&["--undo", m.data["backup"].as_str().unwrap()]).ok);
    assert!(!home.join("my-skills/mine").exists());
    assert_eq!(
        std::fs::read_to_string(claude.join("mine/SKILL.md")).unwrap(),
        "# 我自己写的"
    );

    // 8. 中途失败（Codex 目录写不进去）：已移走的原目录、登记全部恢复，不留备份
    use std::os::unix::fs::PermissionsExt;
    // 同一仓库已登记为 origin（第 5 步）：并进它、沿用它现在的版本，参数一律取预览给出的
    let p = preview("archy");
    let check = p.data["source_check"].clone();
    assert_eq!(p.data["collection"]["exists"], true);
    let mut args: Vec<String> = [
        "--name",
        "archy",
        "--digest",
        check["digest"].as_str().unwrap(),
        "--path",
        check["path"].as_str().unwrap(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some(reference) = check["ref"].as_str() {
        args.extend(["--ref".to_string(), reference.to_string()]);
    }
    if check["same"] == false {
        args.push("--accept-source".into());
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let origin_before = execute(&ctx, &request("skills:status", &[])).data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "skill:origin")
        .unwrap()["spec"]
        .clone();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o555)).unwrap();
    let failed = adopt(&arg_refs);
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!failed.ok);
    assert!(
        failed.errors[0].message.contains("已恢复原样"),
        "{:?}",
        failed.errors
    );
    assert_eq!(tree_digest(&claude.join("archy")), claude_archy_digest);
    assert!(!claude.join("archy").is_symlink() && shared.join("archy/SKILL.md").is_file());
    let status = execute(&ctx, &request("skills:status", &[]));
    let origin_after = status.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "skill:origin")
        .unwrap()["spec"]
        .clone();
    assert_eq!(
        origin_after, origin_before,
        "并进已有来源失败时，来源定义还原"
    );
    let leftovers: Vec<_> = std::fs::read_dir(ctx.data_dir.join("adopt-backups"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            std::fs::read_to_string(e.path().join("manifest.json"))
                .is_ok_and(|m| m.contains("in-progress"))
        })
        .collect();
    assert!(leftovers.is_empty(), "失败并恢复后不留进行中的备份");

    // 9. Claude 自己同步的目录不能纳入
    assert!(!preview("synced").ok);
    let _ = std::fs::remove_dir_all(root);
}
