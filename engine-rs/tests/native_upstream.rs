//! system:refresh-upstream 集成测试：探测宇宙、状态矩阵、缓存纪律、参数反例。
//!
//! 网络探测绝不打真实网络：npm/git 一律经 PATH shim（本文件进程内串行，由
//! PATH_LOCK 保证）。agent bin 探测经 home_dir/.local/bin 假 bin（resolve_bin
//! 的既有测试替身路径），不改 PATH 的测试与改 PATH 的测试不会互相污染。

use agent_manager_engine::{execute, store, Context, Request};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

static PATH_LOCK: Mutex<()> = Mutex::new(());

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    std::env::remove_var("AGENT_MANAGER_LOCK_HELD");
    std::env::remove_var("AGENT_MANAGER_LOCK_PID");
    let base = std::env::temp_dir().join(format!(
        "am-native-upstream-test-{}-{}-{:x}",
        std::process::id(),
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
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
        run_id: "upstream-test-run".to_string(),
    }
}

fn policy_set(ctx: &Context, key: &str, value: Value) {
    let env = execute(
        ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                key,
                "--value",
                &value.to_string(),
            ],
        ),
    );
    assert!(env.ok, "policy:set {key} 失败: {:?}", env.errors);
}

fn set_npm_agent(ctx: &Context, id: &str, bin: &str, package: &str) {
    policy_set(
        ctx,
        &format!("agents.definitions.{id}"),
        json!({
            "display_name": id,
            "bin": bin,
            "preferred_provider": "npm",
            "providers": {"npm": {"install": format!("npm install -g {package}"), "managed": true}}
        }),
    );
}

fn set_mcp_definition(ctx: &Context, name: &str, def: Value) {
    policy_set(ctx, &format!("mcp.definitions.{name}"), def);
}

fn set_skills_repo(ctx: &Context, name: &str, def: Value) {
    policy_set(ctx, &format!("skills.repositories.{name}"), def);
}

fn write_shim(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// home/.local/bin 下的 agent 假 bin：--version 打印指定文本或失败。
fn shim_agent_bin(ctx: &Context, bin: &str, version_line: Option<&str>) {
    let dir = ctx.home_dir.join(".local").join("bin");
    std::fs::create_dir_all(&dir).unwrap();
    match version_line {
        Some(line) => write_shim(&dir, bin, &format!("#!/bin/sh\necho '{line}'\n")),
        None => write_shim(&dir, bin, "#!/bin/sh\nexit 1\n"),
    }
}

/// PATH 前置 shim 目录的守卫；Drop 恢复。
struct PathGuard {
    original: Option<std::ffi::OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl PathGuard {
    fn prepend(dir: &Path) -> Self {
        let lock = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original = std::env::var_os("PATH");
        let mut paths = vec![dir.to_path_buf()];
        if let Some(ref orig) = original {
            paths.extend(std::env::split_paths(orig));
        }
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
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

const HASH_EQUAL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_STALE_LOCAL: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const HASH_STALE_REMOTE: &str = "cccccccccccccccccccccccccccccccccccccccc";
const HASH_NOREPO_REMOTE: &str = "dddddddddddddddddddddddddddddddddddddddd";
const HASH_UVX_REMOTE: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

/// 假 npm：`npm view <pkg> version` 按包名出版本；fail-pkg 失败。
fn npm_shim_body() -> &'static str {
    r#"#!/bin/sh
case "$2" in
  behind-pkg) echo "2.0.0" ;;
  current-pkg) echo "2.0.0" ;;
  ahead-pkg) echo "2.0.0" ;;
  broken-pkg) echo "1.0.0" ;;
  npx-current-pkg) echo "1.5.0" ;;
  npx-nocache-pkg) echo "1.0.0" ;;
  multi-cache-pkg) echo "1.9.0" ;;
  pinned-pkg) echo "1.9.0" ;;
  *) exit 1 ;;
esac
"#
}

/// 假 git：rev-parse 按仓库目录名出本地 HEAD；ls-remote 按 URL 出远端头。
fn git_shim_body() -> String {
    format!(
        r#"#!/bin/sh
case "$1" in
  rev-parse)
    case "$(basename "$PWD")" in
      coll-equal) echo "{HASH_EQUAL}" ;;
      coll-stale) echo "{HASH_STALE_LOCAL}" ;;
      *) exit 1 ;;
    esac ;;
  ls-remote)
    case "$2" in
      https://example.invalid/skills-equal.git) echo "{HASH_EQUAL}	HEAD" ;;
      https://example.invalid/skills-stale.git) echo "{HASH_STALE_REMOTE}	HEAD" ;;
      https://example.invalid/skills-norepo.git) echo "{HASH_NOREPO_REMOTE}	HEAD" ;;
      https://example.invalid/uvx.git) echo "{HASH_UVX_REMOTE}	main" ;;
      *) exit 1 ;;
    esac ;;
  *) exit 1 ;;
esac
"#
    )
}

fn install_shims(base: &Path) -> PathBuf {
    let shim_dir = base.join("shimbin");
    std::fs::create_dir_all(&shim_dir).unwrap();
    write_shim(&shim_dir, "npm", npm_shim_body());
    write_shim(&shim_dir, "git", &git_shim_body());
    shim_dir
}

/// 完整探测宇宙（11 个元素）：agents 五态 + mcp npx/uvx + skills git 三态，
/// 外加不参与探测的反例（disabled / 非 npm / 非 npx/uvx / 无 url / 已停用）。
fn setup_full_universe(ctx: &Context) {
    set_npm_agent(ctx, "behind-agent", "am-test-behind-bin", "behind-pkg");
    set_npm_agent(ctx, "current-agent", "am-test-current-bin", "current-pkg");
    set_npm_agent(ctx, "ahead-agent", "am-test-ahead-bin", "ahead-pkg");
    set_npm_agent(ctx, "broken-agent", "am-test-broken-bin", "broken-pkg");
    set_npm_agent(ctx, "fail-agent", "am-test-fail-bin", "fail-pkg");
    shim_agent_bin(
        ctx,
        "am-test-behind-bin",
        Some("behind-agent version 1.0.0"),
    );
    shim_agent_bin(
        ctx,
        "am-test-current-bin",
        Some("current-agent version 2.0.0"),
    );
    shim_agent_bin(ctx, "am-test-ahead-bin", Some("ahead-agent version 3.0.0"));
    shim_agent_bin(ctx, "am-test-broken-bin", None);
    shim_agent_bin(ctx, "am-test-fail-bin", Some("fail-agent version 1.0.0"));
    // 反例：非 npm provider、disabled npm agent 均不参与探测
    policy_set(
        ctx,
        "agents.definitions.native-agent",
        json!({
            "display_name": "native-agent", "bin": "am-test-native-bin",
            "preferred_provider": "native",
            "providers": {"native": {"install": "curl -fsSL https://example.invalid/install.sh | sh", "managed": true}}
        }),
    );
    set_npm_agent(ctx, "disabled-agent", "am-test-disabled-bin", "behind-pkg");
    policy_set(ctx, "agents.disabled-agent", json!({"enabled": false}));

    // MCP：探测来源是各助手实际配置里的条目（原先是服务库定义，该模型已退役）。
    // 反例：非 npx/uvx 命令不参与探测。停用的条目仍参与（原先按库级开关排除——
    // 库级开关已退役，停用条目重新启用时跑的还是它，版本照样要看）。
    std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
    std::fs::write(
        ctx.home_dir.join(".codex/config.toml"),
        concat!(
            "[mcp_servers.npx-current]\ncommand = \"npx\"\nargs = [\"-y\", \"npx-current-pkg\"]\n",
            "[mcp_servers.npx-nocache]\ncommand = \"npx\"\nargs = [\"-y\", \"npx-nocache-pkg\"]\n",
            "[mcp_servers.uvx-srv]\ncommand = \"uvx\"\nargs = [\"--from\", \"git+https://example.invalid/uvx.git@main\", \"uvx-tool\"]\n",
            "[mcp_servers.plain-node]\ncommand = \"node\"\nargs = [\"server.js\"]\n",
            "[mcp_servers.disabled-npx]\ncommand = \"npx\"\nargs = [\"-y\", \"npx-current-pkg\"]\nenabled = false\n",
        ),
    )
    .unwrap();

    // npx 缓存：仅 npx-current-pkg 有本地已解析版本
    let pkg_dir = ctx
        .home_dir
        .join(".npm/_npx/deadbeef/node_modules/npx-current-pkg");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(pkg_dir.join("package.json"), r#"{"version":"1.5.0"}"#).unwrap();

    set_skills_repo(
        ctx,
        "coll-equal",
        json!({"url": "https://example.invalid/skills-equal.git"}),
    );
    set_skills_repo(
        ctx,
        "coll-stale",
        json!({"url": "https://example.invalid/skills-stale.git"}),
    );
    set_skills_repo(
        ctx,
        "coll-norepo",
        json!({"url": "https://example.invalid/skills-norepo.git"}),
    );
    // 反例：无 url、disabled 集合均不参与探测
    set_skills_repo(ctx, "no-url", json!({"skill_paths": ["x"]}));
    set_skills_repo(
        ctx,
        "disabled-coll",
        json!({"url": "https://example.invalid/disabled.git"}),
    );
    policy_set(ctx, "skills.disabled-coll", json!({"enabled": false}));

    for coll in ["coll-equal", "coll-stale"] {
        std::fs::create_dir_all(ctx.data_dir.join("repositories").join(coll)).unwrap();
    }
}

fn refresh(ctx: &Context, args: &[&str]) -> agent_manager_engine::Envelope {
    execute(ctx, &request("system:refresh-upstream", args))
}

fn probe_table_exists(ctx: &Context) -> bool {
    let Some(conn) = store::open_readonly(ctx).unwrap() else {
        return false;
    };
    conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='upstream_probes'",
        [],
        |row| row.get::<_, i64>(0),
    )
    .unwrap()
        > 0
}

fn cached_row(ctx: &Context, element_id: &str) -> Option<(String, Option<String>, String)> {
    let conn = store::open_readonly(ctx).unwrap()?;
    conn.query_row(
        "SELECT state, upstream, checked_at FROM upstream_probes WHERE element_id = ?1",
        [element_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .ok()
}

fn status_element_freshness(ctx: &Context, command: &str, element_id: &str) -> Value {
    let env = execute(ctx, &request(command, &[]));
    assert!(env.ok, "{command} 失败: {:?}", env.errors);
    env.data["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|el| el["id"] == element_id)
        .unwrap_or_else(|| panic!("{command} 缺少元素 {element_id}"))["freshness"]
        .clone()
}

// 1. fresh 库：三域 status 的 probeable 元素 freshness 全 unknown，status 零写
//    （不建 DB、不建表）；不可探测元素恒 not-applicable。
#[test]
fn fresh_context_status_unknown_and_zero_write() {
    let (base, ctx) = temp_ctx("fresh");

    // 1a. 完全无 DB：内置技能仓库只是推荐来源，不是元素、不带新鲜度（可探测集合的
    //     unknown 语义见 1b 的 skill:coll）；status 不得创建 DB。
    let skills = execute(&ctx, &request("skills:status", &[]));
    assert!(skills.ok, "{:?}", skills.errors);
    assert_eq!(skills.data["elements"], json!([]));
    assert_eq!(skills.data["recommendations"][0]["name"], "agent-skills");
    assert!(skills.data["recommendations"][0].get("freshness").is_none());
    let agents = execute(&ctx, &request("agents:status", &[]));
    assert!(agents.ok);
    for el in agents.data["elements"].as_array().unwrap() {
        let id = el["id"].as_str().unwrap();
        // 声明了 upstream_npm 的助手参与探测（官方安装器装的也算——包名从命令里推不出来，
        // 但 npm 上发的是同一份版本号），没探测过时是 unknown；
        // 没声明又推不出包名的（opencode 是 system provider）才是「没有最新版这个概念」。
        let expected = if matches!(id, "agents:claude-code" | "agents:codex") {
            "unknown"
        } else {
            "not-applicable"
        };
        assert_eq!(el["freshness"]["state"], expected, "{id} 的新鲜度状态");
        assert_eq!(el["freshness"]["checked_at"], Value::Null);
    }
    assert!(!store::db_path(&ctx).exists(), "status 不得创建数据库");

    // 1b. 有 DB（policy:set 首次写入）但从未 refresh：probeable 元素仍全
    //     unknown，且 upstream_probes 表不存在（status 不建表）。
    set_npm_agent(&ctx, "npm-agent", "am-test-npm-bin", "behind-pkg");
    set_mcp_definition(
        &ctx,
        "npx-srv",
        json!({"command": "npx", "args": ["-y", "npx-current-pkg"]}),
    );
    set_skills_repo(
        &ctx,
        "coll",
        json!({"url": "https://example.invalid/skills-equal.git"}),
    );
    assert!(store::db_path(&ctx).exists());

    for (command, id) in [
        ("agents:status", "agents:npm-agent"),
        ("mcp:status", "mcp:npx-srv"),
        ("skills:status", "skill:coll"),
    ] {
        let freshness = status_element_freshness(&ctx, command, id);
        assert_eq!(freshness["state"], "unknown", "{command} {id}");
        assert_eq!(freshness["upstream"], Value::Null, "{command} {id}");
        assert_eq!(freshness["checked_at"], Value::Null, "{command} {id}");
        assert_eq!(freshness["probe"], Value::Null, "{command} {id}");
    }
    // 不可探测元素保持 not-applicable
    set_mcp_definition(&ctx, "plain", json!({"command": "node", "args": ["s.js"]}));
    let freshness = status_element_freshness(&ctx, "mcp:status", "mcp:plain");
    assert_eq!(freshness["state"], "not-applicable");
    assert!(
        !probe_table_exists(&ctx),
        "status 不得创建 upstream_probes 表"
    );

    let _ = std::fs::remove_dir_all(base);
}

// 2. --offline 拒绝（OFFLINE_NETWORK_DENIED），缓存不变；fresh 库上连 DB 都不建。
#[test]
fn offline_is_rejected_and_cache_untouched() {
    let (base, ctx) = temp_ctx("offline");
    let shim_dir = install_shims(&base);
    let _path = PathGuard::prepend(&shim_dir);
    set_npm_agent(&ctx, "behind-agent", "am-test-behind-bin", "behind-pkg");
    shim_agent_bin(
        &ctx,
        "am-test-behind-bin",
        Some("behind-agent version 1.0.0"),
    );

    let env = refresh(&ctx, &[]);
    assert!(env.ok, "{:?}", env.errors);
    let (_, _, checked_at) = cached_row(&ctx, "agents:behind-agent").unwrap();

    let env = refresh(&ctx, &["--offline"]);
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "OFFLINE_NETWORK_DENIED");
    assert!(env.mode.offline);
    assert!(!env.lock.held);
    let (_, _, after) = cached_row(&ctx, "agents:behind-agent").unwrap();
    assert_eq!(checked_at, after, "offline 拒绝不得改写缓存");

    let (fresh_base, fresh_ctx) = temp_ctx("offline-fresh");
    let env = refresh(&fresh_ctx, &["--offline"]);
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "OFFLINE_NETWORK_DENIED");
    assert!(
        !store::db_path(&fresh_ctx).exists(),
        "offline 拒绝发生在任何 DB 写入之前"
    );
    let _ = std::fs::remove_dir_all(fresh_base);
    let _ = std::fs::remove_dir_all(base);
}

// 3. 全状态矩阵：相等→current、落后→behind、本地新→ahead、hash 不同→behind、
//    探测失败→unknown+error 类别且不拖垮整批；summary 计数正确。
#[test]
fn probe_state_matrix_and_summary() {
    let (base, ctx) = temp_ctx("matrix");
    let shim_dir = install_shims(&base);
    let _path = PathGuard::prepend(&shim_dir);
    setup_full_universe(&ctx);

    let env = refresh(&ctx, &[]);
    assert!(env.ok, "{:?}", env.errors);
    assert!(!env.lock.held, "refresh-upstream 不取 apply 锁");
    assert_eq!(env.data["domain"], "system");
    assert_eq!(env.data["verb"], "refresh-upstream");
    assert_eq!(env.data["cache_file"], Value::Null);

    let elements = env.data["elements"].as_array().unwrap();
    let by_id: std::collections::BTreeMap<&str, &Value> = elements
        .iter()
        .map(|el| (el["id"].as_str().unwrap(), el))
        .collect();
    // 14 = 原先的 11 + 两个内置助手（显式声明了 upstream_npm 后一并进探测宇宙）
    // + 停用的 MCP 条目 disabled-npx（库级开关退役，停用条目照样探测）
    assert_eq!(elements.len(), 14, "探测宇宙大小: {elements:?}");
    // 输出按元素 id 字典序稳定排序
    let ids: Vec<&str> = elements
        .iter()
        .map(|el| el["id"].as_str().unwrap())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);

    let case = |id: &str,
                state: &str,
                current: Option<&str>,
                upstream: Option<&str>,
                error: Option<&str>| {
        let el = by_id[id];
        assert_eq!(el["state"], state, "{id}");
        match current {
            Some(v) => assert_eq!(el["current"], v, "{id}"),
            None => assert_eq!(el["current"], Value::Null, "{id}"),
        }
        match upstream {
            Some(v) => assert_eq!(el["upstream"], v, "{id}"),
            None => assert_eq!(el["upstream"], Value::Null, "{id}"),
        }
        match error {
            Some(v) => assert_eq!(el["error"], v, "{id}"),
            None => assert_eq!(el["error"], Value::Null, "{id}"),
        }
    };

    case(
        "agents:behind-agent",
        "behind",
        Some("1.0.0"),
        Some("2.0.0"),
        None,
    );
    case(
        "agents:current-agent",
        "current",
        Some("2.0.0"),
        Some("2.0.0"),
        None,
    );
    case(
        "agents:ahead-agent",
        "ahead",
        Some("3.0.0"),
        Some("2.0.0"),
        None,
    );
    case(
        "agents:broken-agent",
        "unknown",
        None,
        Some("1.0.0"),
        Some("local-version-missing"),
    );
    case(
        "agents:fail-agent",
        "unknown",
        Some("1.0.0"),
        None,
        Some("npm-view-failed"),
    );
    case(
        "mcp:npx-current",
        "current",
        Some("1.5.0"),
        Some("1.5.0"),
        None,
    );
    case(
        "mcp:disabled-npx",
        "current",
        Some("1.5.0"),
        Some("1.5.0"),
        None,
    );
    case(
        "mcp:npx-nocache",
        "unknown",
        None,
        Some("1.0.0"),
        Some("local-version-missing"),
    );
    case(
        "mcp:uvx-srv",
        "unknown",
        None,
        Some(HASH_UVX_REMOTE),
        Some("uvx-local-unavailable"),
    );
    case(
        "skill:coll-equal",
        "current",
        Some(HASH_EQUAL),
        Some(HASH_EQUAL),
        None,
    );
    case(
        "skill:coll-stale",
        "behind",
        Some(HASH_STALE_LOCAL),
        Some(HASH_STALE_REMOTE),
        None,
    );
    case(
        "skill:coll-norepo",
        "unknown",
        None,
        Some(HASH_NOREPO_REMOTE),
        Some("local-head-missing"),
    );

    for el in elements {
        let probe = el["probe"].as_str().unwrap();
        let expected = if el["id"].as_str().unwrap().starts_with("skill:")
            || el["id"].as_str().unwrap() == "mcp:uvx-srv"
        {
            "git-ls-remote"
        } else {
            "npm-view"
        };
        assert_eq!(probe, expected, "{}", el["id"]);
    }

    // 反例元素不在宇宙内
    for excluded in [
        "agents:native-agent",
        "agents:disabled-agent",
        "mcp:plain-node",
        "skill:no-url",
        "skill:disabled-coll",
        "skill:agent-skills",
    ] {
        assert!(!by_id.contains_key(excluded), "{excluded} 不得参与探测");
    }

    assert_eq!(
        env.data["summary"],
        // 两个内置助手声明 upstream_npm 后进了探测宇宙；测试环境的 npm 只认夹具里的包，
        // 它们探测不到结果，计入 unknown
        json!({"probed": 14, "current": 4, "behind": 2, "ahead": 1, "unknown": 7})
    );

    // 缓存落库可回读（含 error 类别），checked_at 已写入
    let (state, upstream, checked_at) = cached_row(&ctx, "skill:coll-stale").unwrap();
    assert_eq!(state, "behind");
    assert_eq!(upstream.as_deref(), Some(HASH_STALE_REMOTE));
    assert!(
        checked_at.ends_with('Z') || checked_at.contains('+'),
        "{checked_at}"
    );

    let _ = std::fs::remove_dir_all(base);
}

// 4. refresh 后重读三域 status：freshness 反映缓存；再次 refresh 更新 checked_at。
#[test]
fn status_reflects_cache_after_refresh() {
    let (base, ctx) = temp_ctx("readback");
    let shim_dir = install_shims(&base);
    let _path = PathGuard::prepend(&shim_dir);
    setup_full_universe(&ctx);

    let env = refresh(&ctx, &[]);
    assert!(env.ok, "{:?}", env.errors);
    let (_, _, first_checked) = cached_row(&ctx, "agents:behind-agent").unwrap();

    let freshness = status_element_freshness(&ctx, "agents:status", "agents:behind-agent");
    assert_eq!(freshness["state"], "behind");
    assert_eq!(freshness["current"], "1.0.0");
    assert_eq!(freshness["upstream"], "2.0.0");
    assert_eq!(freshness["checked_at"], first_checked);
    assert_eq!(freshness["probe"], "npm-view");
    assert!(freshness["cache_age_sec"].as_i64().unwrap() >= 0);

    // 探测失败元素：status 如实回填 unknown + upstream 空
    let freshness = status_element_freshness(&ctx, "agents:status", "agents:fail-agent");
    assert_eq!(freshness["state"], "unknown");
    assert_eq!(freshness["upstream"], Value::Null);

    let freshness = status_element_freshness(&ctx, "mcp:status", "mcp:uvx-srv");
    assert_eq!(freshness["state"], "unknown");
    assert_eq!(freshness["upstream"], HASH_UVX_REMOTE);
    assert_eq!(freshness["current"], Value::Null);
    assert_eq!(freshness["probe"], "git-ls-remote");

    let freshness = status_element_freshness(&ctx, "skills:status", "skill:coll-stale");
    assert_eq!(freshness["state"], "behind");
    assert_eq!(freshness["current"], HASH_STALE_LOCAL);
    assert_eq!(freshness["upstream"], HASH_STALE_REMOTE);

    // 不可探测/不参与元素回填不变
    let freshness = status_element_freshness(&ctx, "mcp:status", "mcp:plain-node");
    assert_eq!(freshness["state"], "not-applicable");
    let freshness = status_element_freshness(&ctx, "skills:status", "skill:disabled-coll");
    assert_eq!(freshness["state"], "not-applicable");
    let freshness = status_element_freshness(&ctx, "agents:status", "agents:disabled-agent");
    assert_eq!(freshness["state"], "not-applicable");

    // 再次 refresh：checked_at 单调不回退
    let env = refresh(&ctx, &[]);
    assert!(env.ok);
    let (_, _, second_checked) = cached_row(&ctx, "agents:behind-agent").unwrap();
    assert!(second_checked >= first_checked);

    let _ = std::fs::remove_dir_all(base);
}

// 5. --only：已知元素只探测该项；未知元素 USAGE_ERROR 且零写。
#[test]
fn only_filters_probe_universe() {
    let (base, ctx) = temp_ctx("only");
    let shim_dir = install_shims(&base);
    let _path = PathGuard::prepend(&shim_dir);
    setup_full_universe(&ctx);

    let env = refresh(&ctx, &["--only", "agents:behind-agent"]);
    assert!(env.ok, "{:?}", env.errors);
    let elements = env.data["elements"].as_array().unwrap();
    assert_eq!(elements.len(), 1);
    assert_eq!(elements[0]["id"], "agents:behind-agent");
    assert_eq!(elements[0]["state"], "behind");
    assert_eq!(env.data["summary"]["probed"], 1);

    // 其他宇宙元素未探测：缓存只有一行，status 仍 unknown
    let conn = store::open_readonly(&ctx).unwrap().unwrap();
    let count: i64 = conn
        .query_row("SELECT count(*) FROM upstream_probes", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    drop(conn);
    let freshness = status_element_freshness(&ctx, "skills:status", "skill:coll-stale");
    assert_eq!(freshness["state"], "unknown");
    assert_eq!(freshness["checked_at"], Value::Null);

    // 未知元素：USAGE_ERROR，不回显取值，零写（不新增缓存行）
    let (fresh_base, fresh_ctx) = temp_ctx("only-unknown");
    set_npm_agent(&fresh_ctx, "npm-agent", "am-test-npm-bin", "behind-pkg");
    let env = refresh(&fresh_ctx, &["--only", "agents:no-such-element"]);
    assert!(!env.ok);
    assert_eq!(env.errors[0].code, "USAGE_ERROR");
    assert!(!env.errors[0].message.contains("no-such-element"));
    assert!(!probe_table_exists(&fresh_ctx));

    let _ = std::fs::remove_dir_all(fresh_base);
    let _ = std::fs::remove_dir_all(base);
}

// 6. 非法参数（位置参数、重复 flag、未知 flag、缺取值、类 flag 取值、dry-run）
//    一律 USAGE_ERROR，错误消息固定不回显取值，且零写。
#[test]
fn illegal_args_are_usage_error_without_echo() {
    let (base, ctx) = temp_ctx("usage");
    set_npm_agent(&ctx, "npm-agent", "am-test-npm-bin", "behind-pkg");

    for (args, sentinel) in [
        (&["positional-token"][..], Some("positional-token")),
        (&["--bogus-flag"][..], Some("--bogus-flag")),
        (
            &["--only", "agents:npm-agent", "--only", "agents:other"][..],
            Some("agents:other"),
        ),
        // --only 缺取值：无取值可回显（消息可指认 flag 名本身）
        (&["--only"][..], None),
        (&["--only", "--offline"][..], Some("--offline")),
        (&["--dry-run"][..], Some("--dry-run")),
    ] {
        let env = refresh(&ctx, args);
        assert!(!env.ok, "args {args:?} 应被拒绝");
        assert_eq!(env.errors[0].code, "USAGE_ERROR", "args {args:?}");
        if let Some(sentinel) = sentinel {
            assert!(
                !env.errors[0].message.contains(sentinel),
                "错误消息不得回显取值 {sentinel}: {}",
                env.errors[0].message
            );
        }
    }
    assert!(!probe_table_exists(&ctx), "参数错误不得产生缓存写入");

    let _ = std::fs::remove_dir_all(base);
}

/// npx 类 MCP 的「本机版本」怎么定。
///
/// 真机上踩到的两件事：
///   - 同一个包在 ~/.npm/_npx 下留着好几份不同版本（npx 按调用参数哈希分目录，
///     见过 8 份）。原先按目录名字典序取第一个，报出来的版本跟实际跑哪个无关——
///     本机最高已有 1.8.0，却报成 1.2.0，看上去像落后了 6 个版本。
///   - 定义里钉死版本（`npx -y foo@1.0.0`）时，包名没剥离版本后缀：缓存目录里
///     找不到 `node_modules/foo@1.0.0`，本机版本恒空；上游又去问
///     `npm view foo@1.0.0 version`，原样把 1.0.0 回给自己，永远比不出落后。
#[test]
fn npx类mcp的本机版本取缓存最高版_钉死版本则以定义为准() {
    let (base, ctx) = temp_ctx("npx-version");
    let shim_dir = install_shims(&base);
    let _path = PathGuard::prepend(&shim_dir);

    set_mcp_definition(
        &ctx,
        "multi",
        json!({"command": "npx", "args": ["-y", "multi-cache-pkg"]}),
    );
    set_mcp_definition(
        &ctx,
        "pinned",
        json!({"command": "npx", "args": ["-y", "pinned-pkg@1.0.0"]}),
    );

    // 同一个包三份缓存，目录名字典序与版本高低故意相反
    for (dir, version) in [("aaaa", "1.2.0"), ("mmmm", "1.8.0"), ("zzzz", "1.4.0")] {
        let pkg_dir = ctx
            .home_dir
            .join(format!(".npm/_npx/{dir}/node_modules/multi-cache-pkg"));
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(
            pkg_dir.join("package.json"),
            format!(r#"{{"version":"{version}"}}"#),
        )
        .unwrap();
    }

    let env = refresh(&ctx, &[]);
    assert!(env.ok, "{:?}", env.errors);

    let multi = status_element_freshness(&ctx, "mcp:status", "mcp:multi");
    assert_eq!(multi["current"], "1.8.0", "要取缓存里最高的那份：{multi}");
    assert_eq!(multi["upstream"], "1.9.0");
    assert_eq!(multi["state"], "behind");

    // 钉死版本：本机没有任何缓存目录，版本仍应来自定义本身
    let pinned = status_element_freshness(&ctx, "mcp:status", "mcp:pinned");
    assert_eq!(pinned["current"], "1.0.0", "钉死的版本就是会跑的版本");
    assert_eq!(
        pinned["upstream"], "1.9.0",
        "上游要问 latest，不是问钉死的那个"
    );
    assert_eq!(pinned["state"], "behind");

    let _ = std::fs::remove_dir_all(base);
}
