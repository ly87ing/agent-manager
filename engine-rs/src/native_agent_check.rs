//! 接入配置的可用性检查：发一个最小的真实请求，一次问清四件事——
//! 地址通不通、令牌有没有效、模型名存不存在、慢不慢。
//!
//! 为什么需要它：写进配置文件不等于能用。地址少个 `/v1`、模型名过期、变量压根没设值，
//! 这三样都要等到下次启动助手、真发一次请求才暴露。这里把那一次请求提前。
//!
//! **只在调用方显式要求时才联网**，其余命令一律零网络。
//!
//! 用 curl 而不是内建 HTTP 客户端：复用上游探测那套外部命令基础设施（超时、进程组隔离、
//! 输出上限都现成），也不为一个诊断动作把 TLS 栈拖进引擎。
//!
//! 令牌不进命令行——`ps` 能看到同机任何进程的参数。请求头与正文都写进 0600 的临时文件，
//! 交给 curl 的 `--config` 读，用完即删。返回、错误与审计里一律不含取值。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::native::{Context, DispatchError, Request};
use crate::native_agent_settings::{self, AuthKind, ConnectionProfile, Kind};

/// 单次检查的上限。慢过这个数的端点，日常用也没法忍。
const TIMEOUT: Duration = Duration::from_secs(20);

/// 探测请求的最小形状：够触发一次真实的模型解析，又不产生可观的用量。
fn probe_body(kind: Kind, model: &str) -> Value {
    match kind {
        Kind::Claude => json!({
            "model": model,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "Hi"}],
        }),
        // Responses 协议的 max_output_tokens 有下限，给 16
        Kind::Codex => json!({
            "model": model,
            "input": "Hi",
            "max_output_tokens": 16,
        }),
    }
}

/// 探测地址：与助手自己拼的一致，拼错了这里就该报出来。
fn probe_endpoint(kind: Kind, base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    match kind {
        Kind::Claude => format!("{base}/v1/messages"),
        Kind::Codex => format!("{base}/responses"),
    }
}

fn headers(kind: Kind, auth: AuthKind, token: &str) -> Vec<String> {
    let mut out = vec!["content-type: application/json".to_string()];
    match kind {
        Kind::Claude => {
            out.push("anthropic-version: 2023-06-01".to_string());
            match auth {
                AuthKind::ApiKey => out.push(format!("x-api-key: {token}")),
                AuthKind::Token => out.push(format!("authorization: Bearer {token}")),
            }
        }
        // Codex 侧一律 Bearer：provider 的 env_key 就是按这个用的
        Kind::Codex => out.push(format!("authorization: Bearer {token}")),
    }
    out
}

/// 0600 写一个临时文件。令牌要落在这里，权限必须先于内容。
fn write_private(path: &Path, content: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(content.as_bytes())?;
    file.sync_all()
}

/// 把 curl config 里的值转义：它按双引号字符串解析，反斜杠与引号要转。
fn cfg_quote(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

struct Probe {
    status: u16,
    latency_ms: u64,
    body: String,
    /// curl 自己失败了（DNS、连不上、超时），压根没拿到响应。
    transport_error: Option<String>,
}

/// 发一次请求。令牌只经由 0600 的 config 文件传给 curl，不进命令行。
fn run_probe(ctx: &Context, url: &str, headers: &[String], body: &Value) -> Probe {
    let stamp = format!(
        "{}-{:x}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let dir = std::env::temp_dir();
    let cfg_path = dir.join(format!("am-check-{stamp}.cfg"));
    let body_path = dir.join(format!("am-check-{stamp}.json"));
    let out_path = dir.join(format!("am-check-{stamp}.out"));

    let cleanup = |paths: &[&PathBuf]| {
        for path in paths {
            let _ = std::fs::remove_file(path);
        }
    };

    let mut cfg = String::new();
    cfg.push_str(&format!("url = {}\n", cfg_quote(url)));
    cfg.push_str("request = \"POST\"\n");
    for header in headers {
        cfg.push_str(&format!("header = {}\n", cfg_quote(header)));
    }
    cfg.push_str(&format!(
        "data-binary = {}\n",
        cfg_quote(&format!("@{}", body_path.display()))
    ));
    cfg.push_str(&format!(
        "output = {}\n",
        cfg_quote(&out_path.display().to_string())
    ));
    cfg.push_str("silent\nshow-error\n");
    cfg.push_str(&format!("max-time = {}\n", TIMEOUT.as_secs()));
    cfg.push_str("write-out = \"%{http_code} %{time_total}\"\n");

    if write_private(&cfg_path, &cfg).is_err()
        || write_private(&body_path, &body.to_string()).is_err()
    {
        cleanup(&[&cfg_path, &body_path, &out_path]);
        return Probe {
            status: 0,
            latency_ms: 0,
            body: String::new(),
            transport_error: Some("无法准备探测请求（临时文件写入失败）".into()),
        };
    }

    let captured = crate::native_upstream::run_capture_public(
        "curl",
        &["--config", &cfg_path.display().to_string()],
        None,
        ctx,
        TIMEOUT + Duration::from_secs(5),
    );

    let response_body = std::fs::read_to_string(&out_path).unwrap_or_default();
    cleanup(&[&cfg_path, &body_path, &out_path]);

    let Some(summary) = captured else {
        return Probe {
            status: 0,
            latency_ms: 0,
            body: response_body,
            transport_error: Some("连不上：地址不对、网络不通，或本机没有 curl".into()),
        };
    };
    let mut parts = summary.split_whitespace();
    let status = parts
        .next()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    let latency_ms = parts
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|secs| (secs * 1000.0) as u64)
        .unwrap_or(0);
    Probe {
        status,
        latency_ms,
        body: response_body,
        transport_error: None,
    }
}

/// 响应体里的错误说明。各家格式不一，按常见形状找，找不到就不编。
fn upstream_message(body: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    let error = parsed.get("error")?;
    let text = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())?;
    // 上游消息原样回显有带回凭据的风险，统一走脱敏
    Some(crate::sanitize::sanitize_diagnostic_message(text))
}

/// 把 HTTP 结果翻成一句能照着做的话。
///
/// 这才是这个功能的价值：知道「401」不如知道「令牌没权限，换一个或去控制台确认」。
fn verdict(kind: Kind, probe: &Probe) -> (bool, String) {
    if let Some(err) = &probe.transport_error {
        return (false, err.clone());
    }
    let upstream = upstream_message(&probe.body);
    let detail = |fallback: &str| match &upstream {
        Some(msg) if !msg.is_empty() => format!("{fallback}：{msg}"),
        _ => fallback.to_string(),
    };
    match probe.status {
        200 => (true, "通了：地址、令牌与模型都没问题".to_string()),
        // 限流说明地址与令牌都是对的，只是这会儿用不了
        429 => (true, detail("地址与令牌都对，但这会儿被限流了")),
        400 => {
            let looks_like_model = upstream
                .as_deref()
                .is_some_and(|m| m.contains("model") || m.contains("模型"));
            if looks_like_model {
                (false, detail("模型名可能不对"))
            } else {
                (false, detail("请求被拒"))
            }
        }
        401 => (false, detail("令牌无效或过期")),
        403 => (false, detail("令牌没有这个权限，或这个模型没开通")),
        404 => (
            false,
            detail(match kind {
                Kind::Claude => "地址或模型名不对（Anthropic 端点通常以 /anthropic 结尾）",
                Kind::Codex => "地址或模型名不对（Codex 端点通常以 /v1 结尾）",
            }),
        ),
        500..=599 => (false, detail("上游服务出错，过会儿再试")),
        0 => (false, "没拿到响应".to_string()),
        other => (false, detail(&format!("返回了 {other}"))),
    }
}

fn kind_of(bin: &str) -> Option<Kind> {
    let name = Path::new(bin).file_stem()?.to_str()?;
    match name {
        "claude" => Some(Kind::Claude),
        "codex" => Some(Kind::Codex),
        _ => None,
    }
}

/// `agents:check --only <助手> [--use <接入配置>]`
///
/// 只读：不碰任何配置文件、不写库（除审计外），也不改生效的接入配置。
pub(crate) fn handle(ctx: &Context, req: &Request) -> Result<Value, DispatchError> {
    let id = req
        .value("--only")
        .ok_or_else(|| DispatchError::new("USAGE_ERROR", "agents:check 需要 --only <助手>"))?
        .trim_start_matches("agents:")
        .to_string();

    let defs = crate::native_agents::load_definitions(ctx)
        .map_err(|err| DispatchError::new("AGENT_ERROR", format!("{err:#}")))?;
    let def = defs
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| DispatchError::new("AGENT_ERROR", format!("没有这个助手：{id}")))?;
    let Some(kind) = kind_of(&def.bin) else {
        return Err(DispatchError::new(
            "AGENT_ERROR",
            "只有 Claude Code 与 Codex 形态的助手能检查接入配置".to_string(),
        ));
    };

    let settings = native_agent_settings::parse(def.spec_ref())
        .map_err(|err| DispatchError::new("AGENT_ERROR", format!("{err:#}")))?;
    // 默认测当前生效的那套；--use 可以在切过去之前先测
    let name = match req.value("--use") {
        Some(explicit) => explicit.to_string(),
        None => settings.active.clone().ok_or_else(|| {
            DispatchError::new(
                "AGENT_ERROR",
                format!("{id} 现在用的是助手自带的账号，没有接入配置可测"),
            )
        })?,
    };
    let profile: &ConnectionProfile = settings.profiles.get(&name).ok_or_else(|| {
        DispatchError::new("AGENT_ERROR", format!("{id} 没有名为 {name} 的接入配置"))
    })?;

    let Some(base_url) = profile.base_url.as_deref() else {
        return Err(DispatchError::new(
            "AGENT_ERROR",
            format!("接入配置 {name} 没填地址，没有可测的端点"),
        ));
    };
    let Some(model) = profile.model.as_deref() else {
        return Err(DispatchError::new(
            "AGENT_ERROR",
            format!("接入配置 {name} 没填模型，检查要靠它才能确认模型名对不对"),
        ));
    };
    let Some(token_var) = profile.token_var.as_deref() else {
        return Err(DispatchError::new(
            "AGENT_ERROR",
            format!("接入配置 {name} 没填令牌变量名，没法带凭据去问"),
        ));
    };
    let Some(token) = crate::secret_values::lookup(token_var) else {
        return Err(DispatchError::new(
            "AGENT_ERROR",
            format!("环境变量 {token_var} 还没有取值：先在密钥页填上，或在启动环境里 export 它"),
        ));
    };

    let endpoint = probe_endpoint(kind, base_url);
    let probe = run_probe(
        ctx,
        &endpoint,
        &headers(kind, profile.auth, &token),
        &probe_body(kind, model),
    );
    let (ok, detail) = verdict(kind, &probe);

    // 审计只记测了谁、结果如何，不记取值，也不记响应正文
    if let Ok(mut conn) = crate::store::open_write(ctx) {
        if let Ok(tx) = conn.transaction() {
            let _ = crate::store::record_operation(
                &tx,
                Some(&req.run_id),
                "agents:check",
                Some(&id),
                Some(&json!({
                    "profile": name,
                    "endpoint": endpoint,
                    "ok": ok,
                    "http_status": probe.status,
                    "latency_ms": probe.latency_ms,
                })),
            );
            let _ = tx.commit();
        }
    }

    let mut out = Map::new();
    out.insert("domain".into(), json!("agents"));
    out.insert("action".into(), json!("check"));
    out.insert("element".into(), json!(format!("agents:{id}")));
    out.insert("profile".into(), json!(name));
    out.insert("endpoint".into(), json!(endpoint));
    out.insert("model".into(), json!(model));
    out.insert("ok".into(), json!(ok));
    out.insert("http_status".into(), json!(probe.status));
    out.insert("latency_ms".into(), json!(probe.latency_ms));
    out.insert("detail".into(), json!(detail));
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(status: u16, body: &str) -> Probe {
        Probe {
            status,
            latency_ms: 10,
            body: body.to_string(),
            transport_error: None,
        }
    }

    #[test]
    fn 端点按各自协议拼_与助手自己拼的一致() {
        assert_eq!(
            probe_endpoint(Kind::Claude, "https://api.deepseek.com/anthropic"),
            "https://api.deepseek.com/anthropic/v1/messages"
        );
        // 末尾斜杠不该拼出双斜杠
        assert_eq!(
            probe_endpoint(Kind::Codex, "https://api.deepseek.com/"),
            "https://api.deepseek.com/responses"
        );
        assert_eq!(
            probe_endpoint(Kind::Codex, "https://api.kimi.com/coding/v1"),
            "https://api.kimi.com/coding/v1/responses"
        );
    }

    #[test]
    fn 两种认证方式发不同的头_codex一律bearer() {
        let bearer = headers(Kind::Claude, AuthKind::Token, "T");
        assert!(bearer.iter().any(|h| h == "authorization: Bearer T"));
        assert!(bearer.iter().any(|h| h.starts_with("anthropic-version:")));

        let api_key = headers(Kind::Claude, AuthKind::ApiKey, "T");
        assert!(api_key.iter().any(|h| h == "x-api-key: T"));
        assert!(!api_key.iter().any(|h| h.starts_with("authorization:")));

        // Codex 的 provider 用 env_key，按 Bearer 发
        let codex = headers(Kind::Codex, AuthKind::ApiKey, "T");
        assert!(codex.iter().any(|h| h == "authorization: Bearer T"));
    }

    #[test]
    fn 状态码翻成能照着做的话() {
        assert!(verdict(Kind::Claude, &probe(200, "")).0);
        // 限流说明地址与令牌都对，算通过
        let (ok, text) = verdict(Kind::Claude, &probe(429, ""));
        assert!(ok);
        assert!(text.contains("限流"));

        let (ok, text) = verdict(
            Kind::Claude,
            &probe(401, r#"{"error":{"message":"invalid key"}}"#),
        );
        assert!(!ok);
        assert!(text.contains("令牌无效"));
        assert!(text.contains("invalid key"), "上游原话要带上：{text}");

        let (_, text) = verdict(Kind::Codex, &probe(404, ""));
        assert!(text.contains("/v1"), "Codex 的 404 要提示端点形状：{text}");
        let (_, text) = verdict(Kind::Claude, &probe(404, ""));
        assert!(text.contains("/anthropic"));

        // 400 且上游提到 model：指向模型名而不是笼统的「请求被拒」
        let (_, text) = verdict(
            Kind::Claude,
            &probe(400, r#"{"error":{"message":"model not found: glm-9"}}"#),
        );
        assert!(text.contains("模型名"), "{text}");
    }

    #[test]
    fn 上游错误里的凭据要脱敏() {
        let leaked = r#"{"error":{"message":"bad Authorization: Bearer sk-secret-value-123"}}"#;
        let (_, text) = verdict(Kind::Claude, &probe(401, leaked));
        assert!(!text.contains("sk-secret-value-123"), "{text}");
    }

    #[test]
    fn 传输失败直接说连不上_不编造状态码() {
        let failed = Probe {
            status: 0,
            latency_ms: 0,
            body: String::new(),
            transport_error: Some("连不上：地址不对、网络不通，或本机没有 curl".into()),
        };
        let (ok, text) = verdict(Kind::Claude, &failed);
        assert!(!ok);
        assert!(text.contains("连不上"));
    }

    #[test]
    fn curl_config_转义_引号与反斜杠不破坏结构() {
        assert_eq!(cfg_quote(r#"a"b"#), r#""a\"b""#);
        assert_eq!(cfg_quote(r"a\b"), r#""a\\b""#);
    }
}
