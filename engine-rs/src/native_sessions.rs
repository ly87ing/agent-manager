//! 最近会话：每个账号最近用过的几个会话，供界面复制「回到这个会话」的命令。
//!
//! 只读、不联网、不写库：按修改时间挑每个账号最新的会话文件，只读文件开头一段，
//! 取会话 id、工作目录和标题。格式按真实 Claude Code 2.1.295 / Codex 0.161.0 的会话文件核对：
//! - Claude：`<配置目录>/projects/<编码后的目录>/<会话 id>.jsonl`。标题取用户改的
//!   `custom-title`（`customTitle`），其次 Claude 自动起的 `ai-title`（`aiTitle`），
//!   都没有时用第一句用户输入。子代理的记录在会话目录下一层，不算会话。
//! - Codex：`<CODEX_HOME>/sessions/**/rollout-*.jsonl`，首行 `session_meta` 带 id、cwd
//!   和来源；来源是子代理（`{"subagent": …}`）的不算。标题取 `session_index.jsonl` 里的
//!   `thread_name`（同一 id 以最后一条为准），没有时用第一句用户输入（跳过注入的
//!   AGENTS.md 与环境说明）。

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{json, Value};

use crate::native::{Context, DispatchError, Request};
use crate::native_usage::{session_roots, SessionKind};

/// 每个账号列出的会话数。
const PER_ACCOUNT: usize = 10;
/// 每个会话文件最多读开头这么多字节：id、目录、标题都在前面。
const HEAD_BYTES: u64 = 256 * 1024;
/// 标题最长字符数。
const TITLE_CHARS: usize = 80;

struct Session {
    id: String,
    cwd: Option<String>,
    title: Option<String>,
    updated_at: SystemTime,
}

/// 文件开头的完整行；最后半行（可能正被写）丢掉。
fn head_lines(path: &Path) -> Vec<Value> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut buf = Vec::new();
    if file.take(HEAD_BYTES).read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    let complete = match buf.iter().rposition(|b| *b == b'\n') {
        Some(end) => &buf[..end],
        None => return Vec::new(),
    };
    String::from_utf8_lossy(complete)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

fn clip(text: &str) -> Option<String> {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    let mut out: String = line.chars().take(TITLE_CHARS).collect();
    if line.chars().count() > TITLE_CHARS {
        out.push('…');
    }
    Some(out)
}

/// 用户输入的文字：字符串，或内容块里的 text / input_text。
fn content_text(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    content.as_array()?.iter().find_map(|block| {
        matches!(
            block.get("type").and_then(Value::as_str),
            Some("text" | "input_text")
        )
        .then(|| block.get("text").and_then(Value::as_str))
        .flatten()
        .map(str::to_string)
    })
}

/// 像人打的话才当标题：斜杠命令的输出、注入的说明都以 `<` 或 `#` 开头。
fn typed_prompt(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('<') || trimmed.starts_with('#') {
        return None;
    }
    clip(trimmed)
}

fn claude_session(path: &Path, updated_at: SystemTime) -> Option<Session> {
    let stem = path.file_stem()?.to_str()?.to_string();
    let mut session = Session {
        id: stem,
        cwd: None,
        title: None,
        updated_at,
    };
    let (mut custom, mut ai, mut prompt) = (None, None, None);
    for line in head_lines(path) {
        if session.cwd.is_none() {
            session.cwd = line.get("cwd").and_then(Value::as_str).map(str::to_string);
        }
        match line.get("type").and_then(Value::as_str) {
            Some("custom-title") => {
                custom = line
                    .get("customTitle")
                    .and_then(Value::as_str)
                    .and_then(clip)
                    .or(custom)
            }
            Some("ai-title") => {
                ai = line
                    .get("aiTitle")
                    .and_then(Value::as_str)
                    .and_then(clip)
                    .or(ai)
            }
            Some("user") if prompt.is_none() && line.get("isMeta") != Some(&Value::Bool(true)) => {
                prompt = line
                    .get("message")
                    .and_then(|message| message.get("content"))
                    .and_then(content_text)
                    .and_then(|text| typed_prompt(&text));
            }
            _ => {}
        }
    }
    session.title = custom.or(ai).or(prompt);
    Some(session)
}

fn codex_titles(root: &Path) -> HashMap<String, String> {
    let mut titles = HashMap::new();
    let Ok(text) = std::fs::read_to_string(root.join("session_index.jsonl")) else {
        return titles;
    };
    for line in text.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let (Some(id), Some(name)) = (
            entry.get("id").and_then(Value::as_str),
            entry
                .get("thread_name")
                .and_then(Value::as_str)
                .and_then(clip),
        ) {
            titles.insert(id.to_string(), name);
        }
    }
    titles
}

fn codex_session(
    path: &Path,
    updated_at: SystemTime,
    titles: &HashMap<String, String>,
) -> Option<Session> {
    let lines = head_lines(path);
    let meta = lines
        .first()
        .filter(|line| line.get("type").and_then(Value::as_str) == Some("session_meta"))?
        .get("payload")?;
    if meta
        .get("source")
        .is_some_and(|source| source.get("subagent").is_some())
    {
        return None;
    }
    let id = meta.get("id").and_then(Value::as_str)?.to_string();
    let prompt = || {
        lines.iter().find_map(|line| {
            let payload = line.get("payload")?;
            (line.get("type").and_then(Value::as_str) == Some("response_item")
                && payload.get("type").and_then(Value::as_str) == Some("message")
                && payload.get("role").and_then(Value::as_str) == Some("user"))
            .then(|| payload.get("content").and_then(content_text))
            .flatten()
            .and_then(|text| typed_prompt(&text))
        })
    };
    Some(Session {
        title: titles.get(&id).cloned().or_else(prompt),
        cwd: meta.get("cwd").and_then(Value::as_str).map(str::to_string),
        id,
        updated_at,
    })
}

/// 账号目录下的会话文件及修改时间，新的在前。
fn newest_files(root: &Path, kind: SessionKind) -> Vec<(PathBuf, SystemTime)> {
    let mut out = Vec::new();
    let modified = |path: &Path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    };
    match kind {
        // 只看 projects/<目录>/ 这一层：再往下是子代理的记录
        SessionKind::Claude => {
            for project in std::fs::read_dir(root.join("projects"))
                .into_iter()
                .flatten()
                .flatten()
            {
                for entry in std::fs::read_dir(project.path())
                    .into_iter()
                    .flatten()
                    .flatten()
                {
                    let path = entry.path();
                    if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                        if let Some(time) = modified(&path) {
                            out.push((path, time));
                        }
                    }
                }
            }
        }
        SessionKind::Codex => {
            let mut stack = vec![root.join("sessions")];
            while let Some(dir) = stack.pop() {
                for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            name.starts_with("rollout-") && name.ends_with(".jsonl")
                        })
                    {
                        if let Some(time) = modified(&path) {
                            out.push((path, time));
                        }
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

fn account_sessions(root: &Path, kind: SessionKind) -> Vec<Session> {
    let titles = match kind {
        SessionKind::Codex => codex_titles(root),
        SessionKind::Claude => HashMap::new(),
    };
    newest_files(root, kind)
        .into_iter()
        .filter_map(|(path, time)| match kind {
            SessionKind::Claude => claude_session(&path, time),
            SessionKind::Codex => codex_session(&path, time, &titles),
        })
        .take(PER_ACCOUNT)
        .collect()
}

fn rfc3339(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// `usage:sessions`：每个账号最近的会话（只读）。
pub(crate) fn sessions(ctx: &Context, _req: &Request) -> Result<Value, DispatchError> {
    Ok(list(ctx))
}

fn list(ctx: &Context) -> Value {
    let accounts: Vec<Value> = session_roots(ctx)
        .into_iter()
        .map(|(root, agent, kind)| {
            let sessions: Vec<Value> = account_sessions(&root, kind)
                .into_iter()
                .map(|session| {
                    json!({
                        "id": session.id,
                        "cwd": session.cwd,
                        "title": session.title,
                        "updated_at": rfc3339(session.updated_at),
                    })
                })
                .collect();
            json!({
                "agent": agent,
                "kind": match kind { SessionKind::Claude => "claude", SessionKind::Codex => "codex" },
                "sessions": sessions,
            })
        })
        .collect();
    json!({ "domain": "usage", "action": "sessions", "accounts": accounts })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, lines: &[Value]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        std::fs::write(path, text).unwrap();
    }

    fn ctx(name: &str) -> (PathBuf, Context) {
        let root =
            std::env::temp_dir().join(format!("am-sessions-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&root);
        (
            root.clone(),
            Context {
                data_dir: root.join("data"),
                home_dir: root.join("home"),
            },
        )
    }

    fn sessions_of<'a>(data: &'a Value, agent: &str) -> &'a Vec<Value> {
        data["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|account| account["agent"] == agent)
            .unwrap()["sessions"]
            .as_array()
            .unwrap()
    }

    #[test]
    fn claude_titles_prefer_rename_then_ai_title_then_first_typed_prompt() {
        let (root, ctx) = ctx("claude");
        let project = ctx.home_dir.join(".claude/projects/-work-app");
        write(
            &project.join("aaa.jsonl"),
            &[
                json!({"type": "user", "cwd": "/work/app", "isMeta": true, "message": {"role": "user", "content": "<local-command-caveat>x"}}),
                json!({"type": "user", "cwd": "/work/app", "message": {"role": "user", "content": "<command-name>/model</command-name>"}}),
                json!({"type": "user", "cwd": "/work/app", "message": {"role": "user", "content": [{"type": "text", "text": "修好登录\n页面的   报错"}]}}),
                json!({"type": "ai-title", "aiTitle": "旧标题", "sessionId": "aaa"}),
                json!({"type": "ai-title", "aiTitle": "登录报错修复", "sessionId": "aaa"}),
            ],
        );
        write(
            &project.join("bbb.jsonl"),
            &[
                json!({"type": "user", "cwd": "/work/app", "message": {"role": "user", "content": "看看测试"}}),
                json!({"type": "ai-title", "aiTitle": "测试", "sessionId": "bbb"}),
                json!({"type": "custom-title", "customTitle": "我的名字", "sessionId": "bbb"}),
            ],
        );
        write(
            &project.join("ccc.jsonl"),
            &[
                json!({"type": "user", "cwd": "/work/app", "message": {"role": "user", "content": "只有一句"}}),
            ],
        );
        // 子代理记录在会话目录下一层，不是会话
        write(
            &project.join("aaa/subagents/agent-1.jsonl"),
            &[
                json!({"type": "user", "cwd": "/work/app", "message": {"role": "user", "content": "子任务"}}),
            ],
        );
        let data = list(&ctx);
        let list = sessions_of(&data, "claude");
        let titles: HashMap<&str, &str> = list
            .iter()
            .map(|s| (s["id"].as_str().unwrap(), s["title"].as_str().unwrap()))
            .collect();
        assert_eq!(list.len(), 3);
        assert_eq!(titles["aaa"], "登录报错修复");
        assert_eq!(titles["bbb"], "我的名字");
        assert_eq!(titles["ccc"], "只有一句");
        assert!(list.iter().all(|s| s["cwd"] == "/work/app"));
        assert!(!ctx.data_dir.exists(), "只读：不建数据目录、不写库");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_skips_subagents_and_titles_from_session_index() {
        let (root, ctx) = ctx("codex");
        let day = ctx.home_dir.join(".codex/sessions/2026/10/08");
        write(
            &day.join("rollout-2026-10-08T10-00-00-main.jsonl"),
            &[
                json!({"type": "session_meta", "payload": {"id": "main", "cwd": "/work/app", "source": "cli"}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "# AGENTS.md instructions for /work/app"}]}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "加个按钮"}]}}),
            ],
        );
        write(
            &day.join("rollout-2026-10-08T11-00-00-named.jsonl"),
            &[
                json!({"type": "session_meta", "payload": {"id": "named", "cwd": "/work/web", "source": "vscode"}}),
            ],
        );
        write(
            &day.join("rollout-2026-10-08T12-00-00-sub.jsonl"),
            &[
                json!({"type": "session_meta", "payload": {"id": "sub", "cwd": "/work/app", "source": {"subagent": "review"}}}),
            ],
        );
        write(
            &ctx.home_dir.join(".codex/session_index.jsonl"),
            &[
                json!({"id": "named", "thread_name": "旧名字"}),
                json!({"id": "named", "thread_name": "网页改版"}),
            ],
        );
        let data = list(&ctx);
        let list = sessions_of(&data, "codex");
        let titles: HashMap<&str, Option<&str>> = list
            .iter()
            .map(|s| (s["id"].as_str().unwrap(), s["title"].as_str()))
            .collect();
        assert_eq!(list.len(), 2);
        assert_eq!(titles["main"], Some("加个按钮"));
        assert_eq!(titles["named"], Some("网页改版"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn newest_first_capped_per_account_and_half_written_last_line_ignored() {
        let (root, ctx) = ctx("cap");
        let project = ctx.home_dir.join(".claude/projects/-p");
        for index in 0..12 {
            let path = project.join(format!("s{index:02}.jsonl"));
            write(
                &path,
                &[
                    json!({"type": "user", "cwd": "/p", "message": {"role": "user", "content": format!("第 {index} 个")}}),
                ],
            );
            let time =
                SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000 + index * 60);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(time)
                .unwrap();
        }
        // 正在写的半行不当作内容
        std::fs::write(
            project.join("s99.jsonl"),
            "{\"type\":\"user\",\"cwd\":\"/p\",\"message\":{\"role\":\"user\",\"content\":\"半",
        )
        .unwrap();
        let data = list(&ctx);
        let list = sessions_of(&data, "claude");
        assert_eq!(list.len(), PER_ACCOUNT);
        assert_eq!(list[0]["id"], "s99");
        assert!(list[0]["title"].is_null());
        assert_eq!(list[1]["id"], "s11");
        assert_eq!(list[1]["updated_at"], "2023-11-14T22:24:20Z");
        let _ = std::fs::remove_dir_all(root);
    }
}
