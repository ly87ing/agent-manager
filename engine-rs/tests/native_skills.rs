use agent_manager_engine::{execute, store, Context, Request};
use fs2::FileExt;
use std::fs::OpenOptions;
use std::path::PathBuf;

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
#[test]
fn skills_builtin_defaults_and_status_without_db() {
    let (root, context) = temp_ctx("skills-builtin");
    let env = execute(&context, &request("skills:status", &[]));
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(env.data["domain"], "skills");
    let elements = env.data["elements"].as_array().unwrap();
    let coll = elements.iter().find(|el| el["id"] == "skill:agent-skills");
    assert!(coll.is_some(), "builtin agent-skills 必须在 status 中呈现");
    let coll = coll.unwrap();
    assert_eq!(
        coll["spec"]["url"],
        "https://github.com/ly87ing/agent-skills.git"
    );
    assert_eq!(coll["policy"]["enabled"], true);
    assert_eq!(coll["state"], "not-materialized");
    // 零写库保证
    assert!(!context.data_dir.exists());
    assert!(!store::db_path(&context).exists());
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
    std::fs::remove_file(&link_2).unwrap();
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
    std::fs::remove_file(skill_dir.join("abs-link")).unwrap();
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
    std::fs::remove_file(skill_dir.join("rel-escape")).unwrap();
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
