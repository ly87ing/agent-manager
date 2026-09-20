//! 用量统计的端到端：扫会话文件 → 按 UTC 小时聚合 → 只读回读。
//!
//! 锁住三件容易算错的事：重复扫不重复计、Codex 的会话累计不能当增量、
//! status 只读不写库。

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

use agent_manager_engine::{execute, store, Context, Request};

fn temp_ctx(tag: &str) -> (PathBuf, Context) {
    let base = std::env::temp_dir().join(format!(
        "am-usage-{}-{}-{:x}",
        std::process::id(),
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&base);
    let ctx = Context {
        data_dir: base.join("data"),
        home_dir: base.join("home"),
    };
    (base, ctx)
}

fn request(command: &str) -> Request {
    Request {
        command: command.to_string(),
        args: vec![],
        run_id: "usage-test".to_string(),
    }
}

fn claude_event(
    id: &str,
    hour: &str,
    model: &str,
    input: i64,
    output: i64,
    cache_r: i64,
) -> String {
    json!({
        "type": "assistant",
        "timestamp": format!("2026-09-20T{hour}:30:00.000Z"),
        "requestId": format!("req_{id}"),
        "message": {
            "id": format!("msg_{id}"),
            "model": model,
            "usage": {
                "input_tokens": input,
                "output_tokens": output,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": cache_r
            }
        }
    })
    .to_string()
}

fn write_claude_session(ctx: &Context, name: &str, lines: &[String]) -> PathBuf {
    let dir = ctx.home_dir.join(".claude/projects/-demo");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
    path
}

fn rows_of(env: &agent_manager_engine::Envelope) -> Vec<Value> {
    env.data["rows"].as_array().cloned().unwrap_or_default()
}

#[test]
fn 扫描按小时与模型聚合_重复扫不重复计() {
    let (base, ctx) = temp_ctx("aggregate");
    write_claude_session(
        &ctx,
        "a.jsonl",
        &[
            claude_event("1", "02", "claude-opus-5", 10, 100, 5000),
            claude_event("2", "02", "claude-opus-5", 20, 200, 6000),
            // 另一个小时、另一个模型各成一桶
            claude_event("3", "03", "claude-haiku-4-5", 5, 50, 0),
        ],
    );

    let scanned = execute(&ctx, &request("usage:scan"));
    assert!(scanned.ok, "{:?}", scanned.errors);
    assert_eq!(scanned.data["files"], 1);

    let status = execute(&ctx, &request("usage:status"));
    let rows = rows_of(&status);
    assert_eq!(rows.len(), 2, "两个桶：{rows:?}");
    let opus = rows
        .iter()
        .find(|r| r["model"] == "claude-opus-5")
        .expect("有 opus 那桶");
    assert_eq!(opus["hour"], "2026-09-20T02:00:00Z");
    assert_eq!(opus["input"], 30);
    assert_eq!(opus["output"], 300);
    assert_eq!(opus["cache_read"], 11000);
    assert_eq!(opus["requests"], 2);

    // 再扫一次：文件没变，一个字节都不该重复计
    let again = execute(&ctx, &request("usage:scan"));
    assert!(again.ok);
    assert_eq!(again.data["files"], 0, "没有新增内容就不该重读");
    let rows = rows_of(&execute(&ctx, &request("usage:status")));
    let opus = rows.iter().find(|r| r["model"] == "claude-opus-5").unwrap();
    assert_eq!(opus["input"], 30, "重复扫描把数字翻倍就是算错了");
    assert_eq!(opus["requests"], 2);

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 新增内容只算增量_同一条消息出现两次只算一次() {
    let (base, ctx) = temp_ctx("incremental");
    let path = write_claude_session(&ctx, "a.jsonl", &[claude_event("1", "02", "m", 10, 100, 0)]);
    assert!(execute(&ctx, &request("usage:scan")).ok);

    // 追加一条新的，外加一条与之前重复的（会话续写时可能重复落盘）
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(&claude_event("2", "02", "m", 7, 70, 0));
    text.push('\n');
    text.push_str(&claude_event("2", "02", "m", 7, 70, 0));
    text.push('\n');
    fs::write(&path, text).unwrap();

    assert!(execute(&ctx, &request("usage:scan")).ok);
    let rows = rows_of(&execute(&ctx, &request("usage:status")));
    let row = &rows[0];
    assert_eq!(row["input"], 17, "10 + 7，重复那条不算：{row:?}");
    assert_eq!(row["requests"], 2);

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn codex_取本次增量_模型名来自前面的上下文事件() {
    let (base, ctx) = temp_ctx("codex");
    let dir = ctx.home_dir.join(".codex/sessions/2026/09/17");
    fs::create_dir_all(&dir).unwrap();
    let lines = vec![
        json!({"timestamp": "2026-09-17T09:00:00Z", "type": "turn_context", "payload": {"model": "gpt-5.6-sol"}}).to_string(),
        json!({
            "timestamp": "2026-09-17T09:10:00Z",
            "type": "event_msg",
            "payload": {"type": "token_count", "info": {
                "total_token_usage": {"input_tokens": 1000, "cached_input_tokens": 0, "output_tokens": 100},
                "last_token_usage": {"input_tokens": 1000, "cached_input_tokens": 400, "cache_write_input_tokens": 0, "output_tokens": 100}
            }}
        })
        .to_string(),
        json!({
            "timestamp": "2026-09-17T09:20:00Z",
            "type": "event_msg",
            "payload": {"type": "token_count", "info": {
                // 累计涨到 3000，但这一次只用了 2000
                "total_token_usage": {"input_tokens": 3000, "cached_input_tokens": 400, "output_tokens": 300},
                "last_token_usage": {"input_tokens": 2000, "cached_input_tokens": 0, "cache_write_input_tokens": 0, "output_tokens": 200}
            }}
        })
        .to_string(),
    ];
    fs::write(
        dir.join("rollout-x.jsonl"),
        format!("{}\n", lines.join("\n")),
    )
    .unwrap();

    assert!(execute(&ctx, &request("usage:scan")).ok);
    let rows = rows_of(&execute(&ctx, &request("usage:status")));
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row["agent"], "codex");
    assert_eq!(row["model"], "gpt-5.6-sol");
    // 第一次 1000 里有 400 命中缓存 → 记 600 input + 400 cache_read；第二次 2000 全新
    assert_eq!(row["input"], 2600);
    assert_eq!(row["cache_read"], 400);
    assert_eq!(row["output"], 300);
    assert_eq!(row["requests"], 2);
    // 拿累计当增量的话 input 会是 4000 以上
    assert_ne!(row["input"], 4000);

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 文件被重写后从头重扫_不会因为偏移过界而漏掉() {
    let (base, ctx) = temp_ctx("rewrite");
    let path = write_claude_session(
        &ctx,
        "a.jsonl",
        &[
            claude_event("1", "02", "m", 10, 100, 0),
            claude_event("2", "02", "m", 10, 100, 0),
        ],
    );
    assert!(execute(&ctx, &request("usage:scan")).ok);

    // 文件被截短（清理/轮转）：旧偏移已越界，应当从头重来
    fs::write(
        &path,
        format!("{}\n", claude_event("9", "05", "m", 1, 2, 0)),
    )
    .unwrap();
    assert!(execute(&ctx, &request("usage:scan")).ok);

    let rows = rows_of(&execute(&ctx, &request("usage:status")));
    let fresh = rows.iter().find(|r| r["hour"] == "2026-09-20T05:00:00Z");
    assert!(fresh.is_some(), "重写后的新内容要被读到：{rows:?}");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn status_只读零写_缺库也不建() {
    let (base, ctx) = temp_ctx("readonly");
    let env = execute(&ctx, &request("usage:status"));
    assert!(env.ok, "{:?}", env.errors);
    assert_eq!(rows_of(&env).len(), 0);
    assert_eq!(env.data["last_scan_at"], Value::Null);
    assert!(!store::db_path(&ctx).exists(), "只读路径不得建库");
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn 清空之后可以从头再扫一遍() {
    let (base, ctx) = temp_ctx("clear");
    write_claude_session(&ctx, "a.jsonl", &[claude_event("1", "02", "m", 10, 100, 0)]);
    assert!(execute(&ctx, &request("usage:scan")).ok);
    assert_eq!(rows_of(&execute(&ctx, &request("usage:status"))).len(), 1);

    let cleared = execute(&ctx, &request("usage:clear"));
    assert!(cleared.ok, "{:?}", cleared.errors);
    assert_eq!(rows_of(&execute(&ctx, &request("usage:status"))).len(), 0);

    // 偏移一并清掉，所以能重新算出同样的数
    assert!(execute(&ctx, &request("usage:scan")).ok);
    let rows = rows_of(&execute(&ctx, &request("usage:status")));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["input"], 10);

    let _ = fs::remove_dir_all(&base);
}
