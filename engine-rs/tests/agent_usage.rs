use agent_manager_engine::{execute, Context, Request};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

/// 额度经 agents:status 的 observed.usage 对外，这里按契约从那里断言。
fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    let base = std::env::temp_dir().join(format!("am-usage-test-{}-{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("home")).unwrap();
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
        run_id: "usage-test".to_string(),
    }
}

fn define_agent(ctx: &Context, id: &str) {
    let value = json!({
        "display_name": id,
        "bin": format!("{id}-not-installed"),
        "preferred_provider": "local-test",
        "providers": {"local-test": {"install": "true"}}
    });
    let env = execute(
        ctx,
        &request(
            "policy:set",
            &[
                "--layer",
                "local",
                "--key",
                &format!("agents.definitions.{id}"),
                "--value",
                &serde_json::to_string(&value).unwrap(),
            ],
        ),
    );
    assert!(env.ok, "定义 agent 失败: {:?}", env.errors);
}

fn usage_of(ctx: &Context, id: &str) -> serde_json::Value {
    let env = execute(ctx, &request("agents:status", &[]));
    assert!(env.ok, "agents:status 失败: {:?}", env.errors);
    let elements = env.data["elements"].as_array().unwrap();
    let element = elements
        .iter()
        .find(|el| el["id"] == json!(format!("agents:{id}")))
        .unwrap_or_else(|| panic!("没有 agents:{id}"));
    element["observed"]["usage"].clone()
}

#[test]
fn claude_额度取自_claude_json_的缓存并跳过没有的窗口() {
    let (base, ctx) = temp_ctx("claude");
    fs::write(
        ctx.home_dir.join(".claude.json"),
        serde_json::to_string(&json!({
            "cachedUsageUtilization": {
                "fetchedAtMs": 1_789_605_697_689i64,
                "utilization": {
                    "five_hour": {"utilization": 8, "resets_at": "2026-09-17T05:30:00.770741+00:00"},
                    "seven_day": {"utilization": 13, "resets_at": "2026-09-21T15:00:00.770771+00:00"},
                    "seven_day_opus": null
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    define_agent(&ctx, "claude-code");

    let usage = usage_of(&ctx, "claude-code");
    assert_eq!(usage["source"], json!(".claude.json"));
    assert_eq!(usage["checked_at"], json!("2026-09-17T00:41:37Z"));
    assert_eq!(
        usage["windows"],
        json!([
            {"window_minutes": 300, "used_percent": 8.0, "resets_at": "2026-09-17T05:30:00Z"},
            {"window_minutes": 10080, "used_percent": 13.0, "resets_at": "2026-09-21T15:00:00Z"}
        ])
    );
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn codex_取最新会话文件的最后一条额度_并能从大文件尾部读到() {
    let (base, ctx) = temp_ctx("codex");
    let day = ctx.home_dir.join(".codex/sessions/2026/09/17");
    fs::create_dir_all(&day).unwrap();

    // 旧会话：不该被选中
    fs::write(
        day.join("rollout-2026-09-17T01-00-00-old.jsonl"),
        format!("{}\n", json!({"payload": {"info": {"rate_limits": {"primary": {"used_percent": 1.0, "window_minutes": 300, "resets_at": 1_789_000_000i64}}}}})),
    )
    .unwrap();

    // 新会话：同一文件里有先后两条，取后一条；中间垫超过尾部扫描窗口的内容，
    // 逼着走「从文件尾部 seek 回读」这条路径
    let mut newer = String::new();
    newer.push_str(&format!("{}\n", json!({"payload": {"info": {"rate_limits": {"primary": {"used_percent": 20.0, "window_minutes": 300, "resets_at": 1i64}}}}})));
    let padding = json!({"payload": {"note": "x".repeat(4096)}}).to_string();
    for _ in 0..400 {
        newer.push_str(&padding);
        newer.push('\n');
    }
    newer.push_str(&format!(
        "{}\n",
        json!({"payload": {"info": {"rate_limits": {
            "primary": {"used_percent": 65.0, "window_minutes": 10080, "resets_at": 1_789_882_199i64},
            "secondary": null,
            "credits": {"balance": "2443.02"},
            "plan_type": "prolite"
        }}}})
    ));
    let newest = day.join("rollout-2026-09-17T09-00-00-new.jsonl");
    fs::write(&newest, &newer).unwrap();
    assert!(fs::metadata(&newest).unwrap().len() > 1 << 20, "垫料不足以触发尾部回读");

    define_agent(&ctx, "codex");

    let usage = usage_of(&ctx, "codex");
    assert_eq!(usage["source"], json!(".codex/sessions"));
    assert_eq!(usage["plan"], json!("prolite"));
    assert_eq!(usage["credits"], json!("2443.02"));
    assert_eq!(
        usage["windows"],
        json!([{"window_minutes": 10080, "used_percent": 65.0, "resets_at": "2026-09-20T05:29:59Z"}])
    );
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 没有本地额度记录的助手为_null_且_status_不写任何文件() {
    let (base, ctx) = temp_ctx("none");
    define_agent(&ctx, "opencode");
    assert_eq!(usage_of(&ctx, "opencode"), json!(null));
    // status 只读：不得凭空生成这些目录
    assert!(!ctx.home_dir.join(".codex").exists());
    assert!(!ctx.home_dir.join(".claude.json").exists());
    let _ = fs::remove_dir_all(&base);
}
