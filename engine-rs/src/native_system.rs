//! Rust 原生 system:audit：只读聚合已实现域
//!（agents/skills/mcp/rules/hooks/aliases/secrets）的 status 元素，加原生
//! runtime/storage 状态，输出冻结的 AuditData 形状
//! （status/version/sections/summary，对齐 app/src/lib/contract.ts）。
//!
//! 不移植旧 Python doctor：不扫描源码配置、不依赖 Python/Git/源码树、不做
//! 全仓 secret scan；version 由构建入口（scripts/build-macos.sh）注入的可信
//! 元数据（完整 HEAD SHA±dirty、commit ISO 日期）经形状与 RFC3339 语义校验
//! 后输出既有 {commit,date} 形状，缺失或畸形 fail-closed 为 null。全程零写：不联网、
//! 不取 apply lock、不写 DB/目录/文件/operation_logs/JSONL；子域失败只降级
//! 对应 section 为固定 error 条目，不中断其他域，原始错误正文不出现在
//! data/envelope。

use serde_json::{json, Map, Value};

use crate::envelope::{diagnostic, Diagnostic};
use crate::native::{Context, Request};
use crate::store;

/// audit 叶子结果：DispatchResult 无法表达“有效 data + errors”，故在叶子
/// 路径返回完整三段，由 execute 的 system:audit 特判直接组装 envelope。
pub(crate) struct AuditOutcome {
    pub data: Value,
    pub warnings: Vec<Diagnostic>,
    pub errors: Vec<Diagnostic>,
}

const WIRED_DOMAINS: &[&str] = &[
    "agents", "skills", "mcp", "rules", "hooks", "aliases", "secrets",
];

const MSG_OK: &str = "状态正常";
const MSG_DISABLED: &str = "已禁用，未纳入管理";
const MSG_UNMANAGED: &str = "检测到非托管目标";
const MSG_UNMATERIALIZED: &str = "托管产物尚未生成";
const MSG_DRIFTED: &str = "托管产物已漂移";
const MSG_UNSUPPORTED: &str = "当前来源或环境不受支持";
const MSG_ABNORMAL: &str = "状态数据异常";
const MSG_DOMAIN_FAILED: &str = "状态查询失败（已脱敏）";
const MSG_RULES_ABSENT: &str = "还没有规则文件";
const MSG_HOOKS_ABSENT: &str = "还没有 Hooks 配置文件";

fn entry(name: &str, status: &str, message: &str, category: Option<&str>) -> Value {
    let mut map = Map::new();
    map.insert("name".into(), Value::String(name.to_string()));
    map.insert("status".into(), Value::String(status.to_string()));
    map.insert("message".into(), Value::String(message.to_string()));
    if let Some(category) = category {
        map.insert("category".into(), Value::String(category.to_string()));
    }
    Value::Object(map)
}

/// 元素状态 → AuditEntry 的稳定映射。name 为经诊断脱敏器处理的 element id；
/// 消息固定中文，不拼接底层错误正文，不输出 spec/observed/path/命令/环境值。
/// 畸形 payload（id 缺失、空字符串或脱敏后为空）固定 error，不进入状态映射。
pub(crate) fn map_element_entry(id: Option<&str>, state: Option<&str>, fallback: &str) -> Value {
    let Some(name) = id
        .map(|raw| crate::sanitize::sanitize_diagnostic_message(raw))
        .filter(|name| !name.is_empty())
    else {
        return entry(fallback, "error", MSG_ABNORMAL, None);
    };
    match state {
        Some("managed-ok") => entry(&name, "ok", MSG_OK, None),
        Some("disabled") => entry(&name, "notice", MSG_DISABLED, None),
        Some("unmanaged-detected") => entry(&name, "notice", MSG_UNMANAGED, None),
        Some("missing") | Some("not-materialized") => {
            entry(&name, "warning", MSG_UNMATERIALIZED, Some("unmaterialized"))
        }
        Some("drifted") => entry(&name, "warning", MSG_DRIFTED, Some("unmaterialized")),
        Some("unsupported") => entry(&name, "warning", MSG_UNSUPPORTED, None),
        _ => entry(&name, "error", MSG_ABNORMAL, None),
    }
}

fn domain_status(ctx: &Context, domain: &str, offline: bool) -> anyhow::Result<Value> {
    let request = Request {
        command: format!("{domain}:status"),
        args: if offline {
            vec!["--offline".to_string()]
        } else {
            Vec::new()
        },
        run_id: String::new(),
    };
    match domain {
        "agents" => crate::native_agents::handle(ctx, &request),
        "skills" => crate::native_skills::handle(ctx, &request),
        "mcp" => crate::native_mcp::handle(ctx, &request),
        "rules" => crate::native_rules::handle(ctx, &request),
        "hooks" => crate::native_hooks::handle(ctx, &request),
        "aliases" => crate::native_aliases::handle(ctx, &request),
        "secrets" => crate::native_secrets::handle(ctx, &request),
        other => anyhow::bail!("未知审计子域 '{other}'"),
    }
}

fn wired_section(ctx: &Context, domain: &str, offline: bool) -> Vec<Value> {
    match domain_status(ctx, domain, offline) {
        Ok(data) => match data.get("elements").and_then(Value::as_array) {
            Some(elements) => elements
                .iter()
                .map(|element| {
                    let state = element.get("state").and_then(Value::as_str);
                    let item =
                        map_element_entry(element.get("id").and_then(Value::as_str), state, domain);
                    // 规则与 hooks 以实际文件为准：没有文件是正常的空状态，不是待生成的产物
                    let absent = match domain {
                        "rules" => Some(MSG_RULES_ABSENT),
                        "hooks" => Some(MSG_HOOKS_ABSENT),
                        _ => None,
                    };
                    if let Some(message) =
                        absent.filter(|_| state == Some("missing") && item["status"] == "warning")
                    {
                        return entry(
                            item["name"].as_str().unwrap_or(domain),
                            "notice",
                            message,
                            None,
                        );
                    }
                    item
                })
                .collect(),
            None => vec![entry(domain, "error", MSG_ABNORMAL, None)],
        },
        Err(_) => vec![entry(domain, "error", MSG_DOMAIN_FAILED, None)],
    }
}

fn runtime_section() -> Vec<Value> {
    vec![entry("engine-rs", "ok", "Rust 原生引擎", None)]
}

fn storage_section(ctx: &Context) -> Vec<Value> {
    let path = store::db_path(ctx);
    if !path.is_file() {
        return vec![entry(
            "native-manager.db",
            "notice",
            "数据库尚未初始化",
            None,
        )];
    }
    match store::open_readonly(ctx) {
        Ok(Some(_)) => vec![entry("native-manager.db", "ok", "数据库就绪", None)],
        Ok(None) => vec![entry(
            "native-manager.db",
            "notice",
            "数据库尚未初始化",
            None,
        )],
        Err(_) => vec![entry("native-manager.db", "error", MSG_DOMAIN_FAILED, None)],
    }
}

/// summary 只统计 error/warning 条目（notice 不计）；warning_categories 只按
/// 确有 category 的 warning 计数到三个固定桶。
pub(crate) fn summarize(sections: &Map<String, Value>) -> (String, Value) {
    let mut errors = 0u64;
    let mut warnings = 0u64;
    let mut optional_tools = 0u64;
    let mut unmaterialized = 0u64;
    let mut official_gaps = 0u64;
    for entries in sections.values().filter_map(Value::as_array) {
        for item in entries {
            let status = item.get("status").and_then(Value::as_str).unwrap_or("");
            let category = item.get("category").and_then(Value::as_str);
            match status {
                "error" => errors += 1,
                "warning" => {
                    warnings += 1;
                    match category {
                        Some("optional_tools") | Some("optional_tool") => optional_tools += 1,
                        Some("unmaterialized") => unmaterialized += 1,
                        Some("official_gaps") | Some("official_gap") => official_gaps += 1,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
    let status = if errors > 0 {
        "error"
    } else if warnings > 0 {
        "warning"
    } else {
        "healthy"
    };
    let summary = json!({
        "errors": errors,
        "warnings": warnings,
        "warning_categories": {
            "optional_tools": optional_tools,
            "unmaterialized": unmaterialized,
            "official_gaps": official_gaps,
        },
    });
    (status.to_string(), summary)
}

/// 严格参数：只接受 bridge 注入所需的可选单个 --offline；重复、未知 flag、
/// 位置参数均 fail-closed，错误消息固定，不回显 token。
fn parse_audit_args(request: &Request) -> Result<bool, Diagnostic> {
    let mut offline = false;
    for arg in &request.args {
        if arg == "--offline" && !offline {
            offline = true;
        } else {
            return Err(diagnostic(
                "USAGE_ERROR",
                None,
                "system:audit 仅接受可选的单个 --offline 参数".to_string(),
            ));
        }
    }
    Ok(offline)
}

/// 发布构建溯源标记：构建入口注入完整 HEAD SHA（dirty 构建带可见 -dirty
/// 后缀，不冒充 clean commit）与 commit ISO 日期；固定前缀加换行结尾保证
/// 最终二进制经 strings 可逐条精确检索。普通 cargo/dev 构建为空值，不代表
/// 任何 commit。标记只含注入值本身，不含源码路径、HOME 或其他环境内容。
#[used]
static BUILD_COMMIT_MARKER: &str = concat!(
    "agent-manager-build-commit:",
    env!("AGENT_MANAGER_BUILD_COMMIT"),
    "\n"
);
#[used]
static BUILD_DATE_MARKER: &str = concat!(
    "agent-manager-build-date:",
    env!("AGENT_MANAGER_BUILD_DATE"),
    "\n"
);

/// 可信 commit 形状：40 位小写 hex（git 输出恒为小写），可选 `-dirty` 后缀。
fn is_trustworthy_commit(commit: &str) -> bool {
    let sha = commit.strip_suffix("-dirty").unwrap_or(commit);
    sha.len() == 40
        && sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// 可信日期：git `%cI` 严格形状（`YYYY-MM-DDTHH:MM:SS±HH:MM` 或 `Z` 结尾，
/// 固定位置与数字位，偏移部分亦须数字）+ RFC3339 语义校验（chrono 解析，
/// 拒绝月份 99、秒 99、2 月 30 等日历或数值非法值）。
fn is_trustworthy_date(date: &str) -> bool {
    let b = date.as_bytes();
    let (body, tz_ok) = match b.len() {
        25 => (
            &b[..19],
            matches!(b[19], b'+' | b'-')
                && b[22] == b':'
                && b[20].is_ascii_digit()
                && b[21].is_ascii_digit()
                && b[23].is_ascii_digit()
                && b[24].is_ascii_digit(),
        ),
        20 => (&b[..19], b[19] == b'Z'),
        _ => return false,
    };
    tz_ok && is_datetime_body(body) && chrono::DateTime::parse_from_rfc3339(date).is_ok()
}

fn is_datetime_body(b: &[u8]) -> bool {
    debug_assert_eq!(b.len(), 19);
    let digit_at = |i: usize| b[i].is_ascii_digit();
    let digits = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18];
    digits.into_iter().all(digit_at)
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
}

/// 由注入元数据构造 AuditData.version：两值齐全且形状可信时返回既有
/// {commit, date} 形状；缺失（普通 cargo/dev 构建）或畸形一律 fail-closed
/// 为 null，对齐契约 version nullable 语义。
fn build_version_from(commit: Option<&str>, date: Option<&str>) -> Value {
    let (Some(commit), Some(date)) = (commit, date) else {
        return Value::Null;
    };
    if !is_trustworthy_commit(commit) || !is_trustworthy_date(date) {
        return Value::Null;
    }
    json!({ "commit": commit, "date": date })
}

fn build_version() -> Value {
    // 引用标记静态，确保发布二进制保留 strings 可检的溯源标记。
    let _retained: (&str, &str) = (BUILD_COMMIT_MARKER, BUILD_DATE_MARKER);
    build_version_from(
        option_env!("AGENT_MANAGER_BUILD_COMMIT"),
        option_env!("AGENT_MANAGER_BUILD_DATE"),
    )
}

pub(crate) fn run(ctx: &Context, request: &Request) -> AuditOutcome {
    let offline = match parse_audit_args(request) {
        Ok(offline) => offline,
        Err(error) => {
            return AuditOutcome {
                data: Value::Object(Map::new()),
                warnings: vec![],
                errors: vec![error],
            }
        }
    };

    let mut sections = Map::new();
    for domain in WIRED_DOMAINS {
        sections.insert(
            (*domain).to_string(),
            Value::Array(wired_section(ctx, domain, offline)),
        );
    }
    sections.insert("runtime".to_string(), Value::Array(runtime_section()));
    sections.insert("storage".to_string(), Value::Array(storage_section(ctx)));

    let (status, summary) = summarize(&sections);
    let data = json!({
        "status": status,
        "version": build_version(),
        "sections": Value::Object(sections),
        "summary": summary,
    });

    let categories = summary["warning_categories"].as_object().expect("object");
    let mut warnings = Vec::new();
    for (key, count) in categories {
        let count = count.as_u64().unwrap_or(0);
        if count > 0 {
            warnings.push(diagnostic(
                &key.to_uppercase(),
                None,
                format!("system:audit 提示项分类 {key}: {count} 项"),
            ));
        }
    }
    let error_count = summary["errors"].as_u64().unwrap_or(0);
    let errors = if error_count > 0 {
        vec![diagnostic(
            "AUDIT_ERRORS",
            None,
            format!("system:audit 报告 {error_count} 个错误"),
        )]
    } else {
        vec![]
    };
    AuditOutcome {
        data,
        warnings,
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_state_mapping_is_stable() {
        let cases: &[(&str, &str, Option<&str>)] = &[
            ("managed-ok", "ok", None),
            ("disabled", "notice", None),
            ("unmanaged-detected", "notice", None),
            ("missing", "warning", Some("unmaterialized")),
            ("not-materialized", "warning", Some("unmaterialized")),
            ("drifted", "warning", Some("unmaterialized")),
            ("unsupported", "warning", None),
        ];
        for (state, expected_status, expected_category) in cases {
            let item = map_element_entry(Some("agents:demo"), Some(state), "fallback");
            assert_eq!(item["status"], *expected_status, "state={state}");
            assert_eq!(item["name"], "agents:demo");
            match expected_category {
                Some(category) => assert_eq!(item["category"], *category, "state={state}"),
                None => assert!(item.get("category").is_none(), "state={state}"),
            }
        }
    }

    #[test]
    fn unknown_or_malformed_states_become_error_entries() {
        for state in [Some("probe-error"), Some(""), Some("weird"), None] {
            let item = map_element_entry(Some("mcp:demo"), state, "fallback");
            assert_eq!(item["status"], "error", "state={state:?}");
        }
    }

    #[test]
    fn malformed_id_always_yields_error_regardless_of_state() {
        // id 缺失/空字符串/脱敏后为空：固定 error + fallback 名称 + 无 category，
        // 任何 state（含 managed-ok/notice 类）都不得改写结论。
        for id in [None, Some("")] {
            for state in [
                Some("managed-ok"),
                Some("disabled"),
                Some("unmanaged-detected"),
                Some("missing"),
                Some("drifted"),
                Some("unsupported"),
                Some("weird"),
                None,
            ] {
                let item = map_element_entry(id, state, "rules");
                assert_eq!(item["name"], "rules", "id={id:?} state={state:?}");
                assert_eq!(item["status"], "error", "id={id:?} state={state:?}");
                assert_eq!(item["message"], MSG_ABNORMAL, "id={id:?} state={state:?}");
                assert!(item.get("category").is_none(), "id={id:?} state={state:?}");
            }
        }
    }

    #[test]
    fn summary_counts_only_error_and_warning_entries() {
        let sections = serde_json::from_value::<Map<String, Value>>(json!({
            "a": [
                {"name": "x", "status": "ok", "message": "m"},
                {"name": "y", "status": "notice", "message": "m"},
                {"name": "z", "status": "warning", "message": "m", "category": "unmaterialized"},
                {"name": "w", "status": "warning", "message": "m"}
            ],
            "b": [{"name": "e", "status": "error", "message": "m"}]
        }))
        .unwrap();
        let (status, summary) = summarize(&sections);
        assert_eq!(status, "error");
        assert_eq!(summary["errors"], 1);
        assert_eq!(summary["warnings"], 2);
        assert_eq!(summary["warning_categories"]["unmaterialized"], 1);
        assert_eq!(summary["warning_categories"]["optional_tools"], 0);
        assert_eq!(summary["warning_categories"]["official_gaps"], 0);
    }

    #[test]
    fn summary_status_thresholds() {
        let healthy = serde_json::from_value::<Map<String, Value>>(json!({
            "a": [
                {"name": "x", "status": "ok", "message": "m"},
                {"name": "y", "status": "notice", "message": "m"}
            ]
        }))
        .unwrap();
        let (status, summary) = summarize(&healthy);
        assert_eq!(status, "healthy");
        assert_eq!(summary["errors"], 0);
        assert_eq!(summary["warnings"], 0);

        let warning = serde_json::from_value::<Map<String, Value>>(json!({
            "a": [{"name": "x", "status": "warning", "message": "m"}]
        }))
        .unwrap();
        assert_eq!(summarize(&warning).0, "warning");
    }

    #[test]
    fn build_version_missing_metadata_stays_null() {
        let sha = "a".repeat(40);
        let date = "2026-09-14T08:46:11+08:00";
        assert_eq!(build_version_from(None, None), Value::Null);
        assert_eq!(build_version_from(Some(&sha), None), Value::Null);
        assert_eq!(build_version_from(None, Some(date)), Value::Null);
        // 构建脚本对缺失环境归一为空串：等同缺失，不得冒充有效身份。
        assert_eq!(build_version_from(Some(""), Some("")), Value::Null);
    }

    #[test]
    fn build_version_accepts_valid_clean_commit() {
        let sha = "8fa4274b".repeat(5);
        let version = build_version_from(Some(&sha), Some("2026-09-14T08:46:11+08:00"));
        assert_eq!(
            version,
            json!({"commit": sha, "date": "2026-09-14T08:46:11+08:00"})
        );
        // Zulu 时区同为 git %cI 合法形状。
        let zulu = build_version_from(Some(&sha), Some("2026-09-14T00:46:11Z"));
        assert_eq!(zulu["date"], "2026-09-14T00:46:11Z");
    }

    #[test]
    fn build_version_marks_dirty_commit_visibly() {
        let sha = "b".repeat(40);
        let dirty = format!("{sha}-dirty");
        let version = build_version_from(Some(&dirty), Some("2026-09-14T08:46:11+08:00"));
        assert_eq!(version["commit"], dirty);
        let commit = version["commit"].as_str().expect("commit string");
        assert!(
            commit.ends_with("-dirty"),
            "dirty 构建必须在 commit 字段可见: {commit}"
        );
    }

    #[test]
    fn build_version_rejects_malformed_metadata() {
        let sha = "c".repeat(40);
        let date = "2026-09-14T08:46:11+08:00";
        for bad_commit in [
            "8fa4274".to_string(),        // 短 sha
            "g".repeat(40),               // 非 hex
            "A".repeat(40),               // 大写 hex
            format!("{sha}-DIRTY"),       // 后缀形状不符
            format!("{sha}-dirty-extra"), // 后缀冗余
            format!("{sha} "),            // 尾部空白
            format!(" {sha}"),            // 前导空白
            format!("{sha}\n"),           // 换行注入
            format!("{sha}\r\nX"),        // CRLF 注入
        ] {
            assert_eq!(
                build_version_from(Some(&bad_commit), Some(date)),
                Value::Null,
                "commit={bad_commit:?}"
            );
        }
        for bad_date in [
            "2026-09-14",
            "2026-09-14 08:46:11",
            "2026-09-14T08:46:11",
            "2026-09-14T08:46:11+0800",
            "not-a-date",
            "",
        ] {
            assert_eq!(
                build_version_from(Some(&sha), Some(bad_date)),
                Value::Null,
                "date={bad_date:?}"
            );
        }
    }

    #[test]
    fn build_version_date_requires_rfc3339_semantics() {
        let sha = "d".repeat(40);
        // 形状接近合法但 RFC3339 语义非法：必须 fail-closed。
        for bad_date in [
            "2026-99-14T08:46:11+08:00", // 月份 99
            "2026-09-99T08:46:11+08:00", // 日 99
            "2026-09-14T99:46:11+08:00", // 时 99
            "2026-09-14T08:99:11+08:00", // 分 99
            "2026-09-14T08:46:99+08:00", // 秒 99
            "2026-09-14T08:46:11+ab:cd", // 偏移非数字
            "2026-09-14T08:46:11+99:00", // 偏移时 99
            "2026-02-30T08:46:11+08:00", // 日历非法（2 月 30）
        ] {
            assert_eq!(
                build_version_from(Some(&sha), Some(bad_date)),
                Value::Null,
                "date={bad_date:?}"
            );
        }
        // 合法 git %cI：数字偏移与 Z 两种形态均输出既有 {commit,date}。
        let offset_form = build_version_from(Some(&sha), Some("2026-09-14T08:28:54+08:00"));
        assert_eq!(offset_form["commit"], sha);
        assert_eq!(offset_form["date"], "2026-09-14T08:28:54+08:00");
        let zulu_form = build_version_from(Some(&sha), Some("2026-09-14T00:28:54Z"));
        assert_eq!(zulu_form["date"], "2026-09-14T00:28:54Z");
    }

    #[test]
    fn build_markers_keep_stable_strings_prefix() {
        // 固定前缀是 verify-macos-dmg.sh 的 strings 精确检索锚点。
        assert!(BUILD_COMMIT_MARKER.starts_with("agent-manager-build-commit:"));
        assert!(BUILD_DATE_MARKER.starts_with("agent-manager-build-date:"));
    }
}
