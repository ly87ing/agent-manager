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

/// `token` 为 None 时不带认证头：只有引用了不需要令牌的接入方式（如本机 ollama）才会这样。
fn headers(kind: Kind, auth: AuthKind, token: Option<&str>) -> Vec<String> {
    let mut out = vec!["content-type: application/json".to_string()];
    match kind {
        Kind::Claude => {
            out.push("anthropic-version: 2023-06-01".to_string());
            match (auth, token) {
                (AuthKind::ApiKey, Some(token)) => out.push(format!("x-api-key: {token}")),
                (AuthKind::Token, Some(token)) => {
                    out.push(format!("authorization: Bearer {token}"))
                }
                (_, None) => {}
            }
        }
        // Codex 侧一律 Bearer：provider 的 env_key 就是按这个用的
        Kind::Codex => {
            if let Some(token) = token {
                out.push(format!("authorization: Bearer {token}"));
            }
        }
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
    run_request(ctx, "POST", url, headers, Some(body), TIMEOUT)
}

/// 发一次请求（`body` 为 None 时不带正文）。令牌只经由 0600 的 config 文件传给 curl，不进命令行。
fn run_request(
    ctx: &Context,
    method: &str,
    url: &str,
    headers: &[String],
    body: Option<&Value>,
    timeout: Duration,
) -> Probe {
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
    cfg.push_str(&format!("request = {}\n", cfg_quote(method)));
    for header in headers {
        cfg.push_str(&format!("header = {}\n", cfg_quote(header)));
    }
    if body.is_some() {
        cfg.push_str(&format!(
            "data-binary = {}\n",
            cfg_quote(&format!("@{}", body_path.display()))
        ));
    }
    cfg.push_str(&format!(
        "output = {}\n",
        cfg_quote(&out_path.display().to_string())
    ));
    cfg.push_str("silent\nshow-error\n");
    cfg.push_str(&format!("max-time = {}\n", timeout.as_secs()));
    cfg.push_str("write-out = \"%{http_code} %{time_total}\"\n");

    if write_private(&cfg_path, &cfg).is_err()
        || body.is_some_and(|body| write_private(&body_path, &body.to_string()).is_err())
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
        timeout + Duration::from_secs(5),
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
    // 返回的是网页不是接口响应：多半被防火墙 / Cloudflare 这类防护拦下，或地址填成了网站首页
    if probe.body.trim_start().starts_with('<') && probe.status != 200 {
        return (
            false,
            format!(
                "返回了网页而不是接口响应（{}）：可能被防火墙或 Cloudflare 这类防护拦下，或地址不是接口地址",
                probe.status
            ),
        );
    }
    // 只收官方编码助手客户端请求的端点（如 Kimi Code 会员）：检查不冒充客户端，如实说明验不了
    let coding_agents_only = probe.status == 403
        && [upstream.as_deref().unwrap_or(""), probe.body.as_str()]
            .iter()
            .any(|text| text.to_ascii_lowercase().contains("coding agent"));
    if coding_agents_only {
        return (
            false,
            "这个端点只接受编码助手客户端自己发的请求，无法代为检查；请直接在终端里用助手验证"
                .to_string(),
        );
    }
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
    let only = req.value("--only").ok_or_else(|| {
        DispatchError::new(
            "USAGE_ERROR",
            "agents:check 需要 --only <助手 | profiles:独立环境>",
        )
    })?;
    if let Some(profile_id) = only.strip_prefix("profiles:") {
        return handle_profile(ctx, req, profile_id);
    }
    let id = only.trim_start_matches("agents:").to_string();

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

    // 引用型接入（Codex）：地址与令牌变量以 config.toml 里被引用的那张表为准，内置接入方式用已知默认值
    let (base_url, token_var) = match profile.provider_ref.as_deref() {
        Some(_) if kind == Kind::Claude => {
            return Err(DispatchError::new(
                "AGENT_ERROR",
                format!("接入配置 {name} 引用了 config.toml 的接入方式，只适用于 Codex"),
            ));
        }
        Some(reference) => {
            native_agent_settings::codex_referenced_endpoint(ctx, def.config_path(), reference)
                .map_err(|err| {
                    DispatchError::new("AGENT_ERROR", format!("接入配置 {name}：{err:#}"))
                })?
        }
        None => {
            let Some(base_url) = profile.base_url.clone() else {
                return Err(DispatchError::new(
                    "AGENT_ERROR",
                    format!("接入配置 {name} 没填地址，没有可测的端点"),
                ));
            };
            (base_url, profile.token_var.clone())
        }
    };
    let Some(model) = profile.model.as_deref() else {
        return Err(DispatchError::new(
            "AGENT_ERROR",
            format!("接入配置 {name} 没填模型，检查要靠它才能确认模型名对不对"),
        ));
    };
    // 自己填的接入必须带令牌变量；引用的接入方式本来就不需要令牌时（本机 ollama 等）不带认证头
    if token_var.is_none() && profile.provider_ref.is_none() {
        return Err(DispatchError::new(
            "AGENT_ERROR",
            format!("接入配置 {name} 没填令牌变量名，没法带凭据去问"),
        ));
    }
    let token = match token_var.as_deref() {
        Some(token_var) => Some(crate::secret_values::lookup(token_var).ok_or_else(|| {
            DispatchError::new(
                "AGENT_ERROR",
                format!(
                    "环境变量 {token_var} 还没有取值：先在密钥页填上，或在启动环境里 export 它"
                ),
            )
        })?),
        None => None,
    };

    let mut out = probe_and_report(
        ctx,
        req,
        &format!("agents:{id}"),
        Some(&name),
        kind,
        &base_url,
        profile.auth,
        token.as_deref(),
        model,
    );
    out.insert("profile".into(), json!(name));
    Ok(Value::Object(out))
}

/// 发一次检查请求、翻译结果并记审计：接入配置与独立环境共用。审计只记测了谁、结果如何，
/// 不记取值，也不记响应正文。
#[allow(clippy::too_many_arguments)]
fn probe_and_report(
    ctx: &Context,
    req: &Request,
    element: &str,
    profile: Option<&str>,
    kind: Kind,
    base_url: &str,
    auth: AuthKind,
    token: Option<&str>,
    model: &str,
) -> Map<String, Value> {
    let endpoint = probe_endpoint(kind, base_url);
    let probe = run_probe(
        ctx,
        &endpoint,
        &headers(kind, auth, token),
        &probe_body(kind, model),
    );
    let (ok, detail) = verdict(kind, &probe);

    if let Ok(mut conn) = crate::store::open_write(ctx) {
        if let Ok(tx) = conn.transaction() {
            let _ = crate::store::record_operation(
                &tx,
                Some(&req.run_id),
                "agents:check",
                Some(element.trim_start_matches("agents:")),
                Some(&json!({
                    "profile": profile,
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
    out.insert("element".into(), json!(element));
    out.insert("endpoint".into(), json!(endpoint));
    out.insert("model".into(), json!(model));
    out.insert("ok".into(), json!(ok));
    out.insert("http_status".into(), json!(probe.status));
    out.insert("latency_ms".into(), json!(probe.latency_ms));
    out.insert("detail".into(), json!(detail));
    out
}

/// 独立环境要检查的端点：按实例定义（与写进实例配置的一致），凭据从密钥来源解析。
struct ProfileEndpoint {
    kind: Kind,
    base_url: String,
    auth: AuthKind,
    token_var: String,
    model: Option<String>,
}

/// 从实例定义取出检查要用的地址、凭据变量与模型。官方账号环境没有第三方端点，不代为发请求。
fn profile_endpoint(
    def: &crate::native_profiles::model::ProfileDef,
) -> Result<ProfileEndpoint, String> {
    use crate::native_profiles::model::ProfileKind;
    let reference = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .and_then(|text| text.strip_prefix("${"))
            .and_then(|text| text.strip_suffix('}'))
            .map(str::to_string)
    };
    let official = || {
        format!(
            "独立环境 {} 用官方账号，检查不代为发请求；请在终端里用它的启动命令验证",
            def.id
        )
    };
    let (kind, base_url, auth, token_var) = match def.kind {
        ProfileKind::Claude => {
            let Some(base_url) = def
                .env
                .get("ANTHROPIC_BASE_URL")
                .and_then(Value::as_str)
                .filter(|url| !url.trim().is_empty())
            else {
                return Err(official());
            };
            let (auth, var) = match (
                reference(def.env.get("ANTHROPIC_AUTH_TOKEN")),
                reference(def.env.get("ANTHROPIC_API_KEY")),
            ) {
                (Some(var), _) => (AuthKind::Token, var),
                (None, Some(var)) => (AuthKind::ApiKey, var),
                (None, None) => {
                    return Err(format!(
                        "独立环境 {} 没有引用凭据变量，没法带凭据去问",
                        def.id
                    ))
                }
            };
            (Kind::Claude, base_url.to_string(), auth, var)
        }
        ProfileKind::Codex => {
            let Some(base_url) = def.base_url.clone() else {
                return Err(official());
            };
            let Some(var) = def.env_key.clone() else {
                return Err(format!("独立环境 {} 没有凭据变量，没法带凭据去问", def.id));
            };
            (Kind::Codex, base_url, AuthKind::Token, var)
        }
    };
    let model = def
        .model
        .clone()
        .or_else(|| {
            def.env
                .get("ANTHROPIC_MODEL")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|model| !model.trim().is_empty());
    Ok(ProfileEndpoint {
        kind,
        base_url,
        auth,
        token_var,
        model,
    })
}

/// `agents:check --only profiles:<独立环境>`：检查独立环境的第三方端点。只读，除审计外不写。
fn handle_profile(ctx: &Context, req: &Request, id: &str) -> Result<Value, DispatchError> {
    let policy = crate::store::effective_policy(ctx)
        .map_err(|err| DispatchError::new("AGENT_ERROR", format!("{err:#}")))?;
    let def = crate::native_profiles::model::load_definition_entries(&policy)
        .into_iter()
        .find(|(entry_id, _, _)| entry_id == id)
        .ok_or_else(|| DispatchError::new("AGENT_ERROR", format!("没有这个独立环境：{id}")))?
        .2
        .map_err(|err| DispatchError::new("AGENT_ERROR", format!("{err:#}")))?;
    let endpoint = profile_endpoint(&def).map_err(|err| DispatchError::new("AGENT_ERROR", err))?;
    let model = endpoint.model.clone().ok_or_else(|| {
        DispatchError::new(
            "AGENT_ERROR",
            format!("独立环境 {id} 没填模型，检查要靠它才能确认模型名对不对"),
        )
    })?;
    let token = crate::secret_values::lookup(&endpoint.token_var).ok_or_else(|| {
        DispatchError::new(
            "AGENT_ERROR",
            format!(
                "环境变量 {} 还没有取值：先在密钥页填上，或在启动环境里 export 它",
                endpoint.token_var
            ),
        )
    })?;
    let out = probe_and_report(
        ctx,
        req,
        &format!("profiles:{id}"),
        None,
        endpoint.kind,
        &endpoint.base_url,
        endpoint.auth,
        Some(&token),
        &model,
    );
    Ok(Value::Object(out))
}
/// 模型列表的单次请求上限：列模型不该比真正对话还慢。
const MODELS_TIMEOUT: Duration = Duration::from_secs(15);

/// 列模型要试的地址，都在接入地址的同一主机下（凭据不发往别处）。Claude 形态先按 Anthropic
/// 协议的 `{base}/v1/models`，再去掉 `/anthropic` 这类兼容后缀试同一厂商的 OpenAI 协议列表；
/// Codex 形态的地址通常已带 `/v1`。
fn model_list_urls(kind: Kind, base_url: &str) -> Vec<String> {
    let base = base_url.trim_end_matches('/');
    let mut urls = Vec::new();
    match kind {
        Kind::Claude => {
            urls.push(format!("{base}/v1/models"));
            urls.push(format!("{base}/models"));
            for suffix in ["/apps/anthropic", "/api/anthropic", "/anthropic"] {
                if let Some(root) = base.strip_suffix(suffix) {
                    urls.push(format!("{root}/v1/models"));
                    urls.push(format!("{root}/models"));
                    break;
                }
            }
        }
        Kind::Codex => {
            urls.push(format!("{base}/models"));
            if !base.ends_with("/v1") {
                urls.push(format!("{base}/v1/models"));
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    urls.retain(|url| seen.insert(url.clone()));
    urls
}

/// 从模型列表响应里取出模型 id：OpenAI 与 Anthropic 都是 `{"data": [{"id": …}]}`，也认
/// `{"models": [...]}` 与纯字符串数组。去重、保持顺序，最多 500 个；取不出返回 None。
fn model_ids(body: &str) -> Option<Vec<String>> {
    let value: Value = serde_json::from_str(body).ok()?;
    let items = value
        .get("data")
        .or_else(|| value.get("models"))
        .unwrap_or(&value)
        .as_array()?;
    let mut seen = std::collections::HashSet::new();
    let ids: Vec<String> = items
        .iter()
        .filter_map(|item| {
            item.as_str().or_else(|| {
                ["id", "name", "model"]
                    .iter()
                    .find_map(|key| item.get(*key).and_then(Value::as_str))
            })
        })
        .map(str::trim)
        .filter(|id| !id.is_empty() && id.len() <= 200 && !id.chars().any(char::is_control))
        .filter(|id| seen.insert(id.to_string()))
        .take(500)
        .map(str::to_string)
        .collect();
    (!ids.is_empty()).then_some(ids)
}

/// 列模型要用的端点与凭据变量。
struct ModelsTarget {
    element: String,
    kind: Kind,
    base_url: String,
    token_var: Option<String>,
}

fn models_target(ctx: &Context, req: &Request) -> Result<ModelsTarget, DispatchError> {
    let error = |message: String| DispatchError::new("AGENT_ERROR", message);
    // 表单里还没保存的接入：地址、凭据变量名与形态随请求给出，凭据取值仍从密钥来源解析
    if let Some(raw) = req.value("--draft") {
        let draft: Value = serde_json::from_str(raw)
            .map_err(|_| DispatchError::new("USAGE_ERROR", "--draft 必须是 JSON 对象"))?;
        let text = |key: &str| {
            draft
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        let kind = match text("kind").as_deref() {
            Some("claude") => Kind::Claude,
            Some("codex") => Kind::Codex,
            _ => return Err(error("--draft 的 kind 只能是 claude 或 codex".into())),
        };
        let base_url = text("base_url").ok_or_else(|| error("先填接口地址".into()))?;
        if !(base_url.starts_with("https://") || base_url.starts_with("http://"))
            || crate::native::url_has_userinfo(&base_url)
        {
            return Err(error(
                "接口地址只允许 https:// 或 http://，且不能带账号密码".into(),
            ));
        }
        let token_var = text("token_var").ok_or_else(|| error("先填凭据变量名".into()))?;
        if !crate::secret_values::valid_var_name(&token_var) {
            return Err(error("凭据变量名不合法".into()));
        }
        return Ok(ModelsTarget {
            element: "draft".into(),
            kind,
            base_url,
            token_var: Some(token_var),
        });
    }
    let only = req.value("--only").ok_or_else(|| {
        DispatchError::new(
            "USAGE_ERROR",
            "agents:models 需要 --only <助手> --use <接入配置>、--only profiles:<独立环境> 或 --draft <JSON>",
        )
    })?;
    if let Some(id) = only.strip_prefix("profiles:") {
        let policy =
            crate::store::effective_policy(ctx).map_err(|err| error(format!("{err:#}")))?;
        let def = crate::native_profiles::model::load_definition_entries(&policy)
            .into_iter()
            .find(|(entry_id, _, _)| entry_id == id)
            .ok_or_else(|| error(format!("没有这个独立环境：{id}")))?
            .2
            .map_err(|err| error(format!("{err:#}")))?;
        let endpoint = profile_endpoint(&def).map_err(error)?;
        return Ok(ModelsTarget {
            element: format!("profiles:{id}"),
            kind: endpoint.kind,
            base_url: endpoint.base_url,
            token_var: Some(endpoint.token_var),
        });
    }
    let id = only.trim_start_matches("agents:").to_string();
    let defs =
        crate::native_agents::load_definitions(ctx).map_err(|err| error(format!("{err:#}")))?;
    let def = defs
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| error(format!("没有这个助手：{id}")))?;
    let kind = kind_of(&def.bin)
        .ok_or_else(|| error("只有 Claude Code 与 Codex 形态的助手能列模型".into()))?;
    let name = req.value("--use").ok_or_else(|| {
        DispatchError::new("USAGE_ERROR", "列接入配置的模型要带 --use <接入配置>")
    })?;
    let settings =
        native_agent_settings::parse(def.spec_ref()).map_err(|err| error(format!("{err:#}")))?;
    let profile = settings
        .profiles
        .get(name)
        .ok_or_else(|| error(format!("{id} 没有名为 {name} 的接入配置")))?;
    let (base_url, token_var) = match profile.provider_ref.as_deref() {
        Some(reference) if kind == Kind::Codex => {
            native_agent_settings::codex_referenced_endpoint(ctx, def.config_path(), reference)
                .map_err(|err| error(format!("接入配置 {name}：{err:#}")))?
        }
        Some(_) => {
            return Err(error(format!(
                "接入配置 {name} 引用了 config.toml 的接入方式，只适用于 Codex"
            )))
        }
        None => (
            profile
                .base_url
                .clone()
                .ok_or_else(|| error(format!("接入配置 {name} 没填地址")))?,
            profile.token_var.clone(),
        ),
    };
    Ok(ModelsTarget {
        element: format!("agents:{id}"),
        kind,
        base_url,
        token_var,
    })
}

/// `agents:models`：问接口这把凭据能用哪些模型，给表单挑模型名用。
///
/// 与检查一样只在用户点它时才联网、只读（除审计外不写）；凭据经 0600 临时文件交给 curl，
/// 只发往接入地址的同一主机。试完候选地址都拿不到列表时如实说明，不编模型名。
pub(crate) fn handle_models(ctx: &Context, req: &Request) -> Result<Value, DispatchError> {
    let target = models_target(ctx, req)?;
    let token = match target.token_var.as_deref() {
        Some(var) => Some(crate::secret_values::lookup(var).ok_or_else(|| {
            DispatchError::new(
                "AGENT_ERROR",
                format!(
                    "环境变量 {var} 还没有取值：先在密钥页填上（或保存接入时一并填写），再获取模型列表"
                ),
            )
        })?),
        None => None,
    };
    let mut headers = vec!["accept: application/json".to_string()];
    if let Some(token) = token.as_deref() {
        // 与官方 apiKeyHelper 的做法一致：同一把凭据同时放进 x-api-key 与 Bearer，
        // Anthropic 协议与 OpenAI 协议的列表接口各认其一
        if target.kind == Kind::Claude {
            headers.push(format!("x-api-key: {token}"));
        }
        headers.push(format!("authorization: Bearer {token}"));
    }
    if target.kind == Kind::Claude {
        headers.push("anthropic-version: 2023-06-01".to_string());
    }

    let urls = model_list_urls(target.kind, &target.base_url);
    let mut outcome: Result<(String, Vec<String>), String> = Err(format!(
        "这个接口没有提供模型列表（试过 {} 个地址）；模型名请按厂商文档填写",
        urls.len()
    ));
    let mut last_status = 0;
    for url in &urls {
        let probe = run_request(ctx, "GET", url, &headers, None, MODELS_TIMEOUT);
        last_status = probe.status;
        if probe.transport_error.is_some() {
            outcome = Err(verdict(target.kind, &probe).1);
            break;
        }
        match probe.status {
            200 => {
                if let Some(ids) = model_ids(&probe.body) {
                    outcome = Ok((url.clone(), ids));
                    break;
                }
            }
            // 这个地址没有列表接口：换下一个
            404 | 405 => {}
            _ => {
                let (_, detail) = verdict(target.kind, &probe);
                outcome = Err(format!("获取模型列表失败：{detail}"));
                break;
            }
        }
    }

    if let Ok(mut conn) = crate::store::open_write(ctx) {
        if let Ok(tx) = conn.transaction() {
            let _ = crate::store::record_operation(
                &tx,
                Some(&req.run_id),
                "agents:models",
                Some(target.element.trim_start_matches("agents:")),
                Some(&json!({
                    "endpoint": outcome.as_ref().ok().map(|(url, _)| url.clone()),
                    "count": outcome.as_ref().map(|(_, ids)| ids.len()).unwrap_or(0),
                    "http_status": last_status,
                })),
            );
            let _ = tx.commit();
        }
    }

    let (endpoint, models) =
        outcome.map_err(|message| DispatchError::new("AGENT_ERROR", message))?;
    Ok(json!({
        "domain": "agents",
        "action": "models",
        "element": target.element,
        "endpoint": endpoint,
        "count": models.len(),
        "models": models,
    }))
}

/// 有官方余额查询接口的厂商：按接入地址的主机认出来，余额接口与接入地址同主机（凭据不发往别处）。
/// 只收厂商 API 文档里写明的接口：DeepSeek「查询余额」`GET /user/balance`，Moonshot（Kimi 开放平台）
/// 「查询余额」`GET /v1/users/me/balance`。套餐额度一类没有公开文档的接口不收。
fn balance_endpoint(base_url: &str) -> Option<(&'static str, String)> {
    let rest = base_url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?.to_ascii_lowercase();
    match host.as_str() {
        "api.deepseek.com" => Some(("deepseek", "https://api.deepseek.com/user/balance".into())),
        "api.moonshot.cn" | "api.moonshot.ai" => {
            Some(("moonshot", format!("https://{host}/v1/users/me/balance")))
        }
        _ => None,
    }
}

/// 余额响应 → `[{label, amount, currency}]`。字段按两家文档；取不出返回 None，不编数字。
fn balance_items(vendor: &str, body: &str) -> Option<Vec<Value>> {
    let value: Value = serde_json::from_str(body).ok()?;
    let number = |v: Option<&Value>| -> Option<String> {
        let v = v?;
        v.as_str()
            .map(str::to_string)
            .or_else(|| v.as_f64().map(|n| format!("{n}")))
    };
    let items: Vec<Value> = match vendor {
        "deepseek" => value
            .get("balance_infos")?
            .as_array()?
            .iter()
            .filter_map(|info| {
                Some(json!({
                    "label": "余额",
                    "amount": number(info.get("total_balance"))?,
                    "currency": info.get("currency").and_then(Value::as_str),
                }))
            })
            .collect(),
        "moonshot" => {
            let data = value.get("data")?;
            [
                ("可用余额", "available_balance"),
                ("代金券", "voucher_balance"),
                ("现金余额", "cash_balance"),
            ]
            .iter()
            .filter_map(|(label, key)| {
                Some(json!({"label": label, "amount": number(data.get(*key))?, "currency": null}))
            })
            .collect()
        }
        _ => return None,
    };
    (!items.is_empty()).then_some(items)
}

/// `agents:balance`：查这套接入（或独立环境、表单草稿）凭据的账户余额。只对有官方余额接口的厂商
/// 可用；与检查同一口径：只在点它时联网、只读（除审计外不写），凭据经 0600 临时文件交给 curl。
pub(crate) fn handle_balance(ctx: &Context, req: &Request) -> Result<Value, DispatchError> {
    let target = models_target(ctx, req)?;
    let (vendor, url) = balance_endpoint(&target.base_url).ok_or_else(|| {
        DispatchError::new(
            "AGENT_ERROR",
            "这家接口没有公开的余额查询接口，请到厂商控制台查看",
        )
    })?;
    let var = target
        .token_var
        .as_deref()
        .ok_or_else(|| DispatchError::new("AGENT_ERROR", "没有凭据变量，没法查余额"))?;
    let token = crate::secret_values::lookup(var).ok_or_else(|| {
        DispatchError::new(
            "AGENT_ERROR",
            format!("环境变量 {var} 还没有取值：先在密钥页填上，再查余额"),
        )
    })?;
    let headers = vec![
        "accept: application/json".to_string(),
        format!("authorization: Bearer {token}"),
    ];
    let probe = run_request(ctx, "GET", &url, &headers, None, MODELS_TIMEOUT);
    let outcome = if probe.transport_error.is_some() || probe.status != 200 {
        Err(format!("查余额失败：{}", verdict(target.kind, &probe).1))
    } else {
        balance_items(vendor, &probe.body)
            .ok_or_else(|| "余额接口的返回看不懂，请到厂商控制台查看".to_string())
    };

    if let Ok(mut conn) = crate::store::open_write(ctx) {
        if let Ok(tx) = conn.transaction() {
            let _ = crate::store::record_operation(
                &tx,
                Some(&req.run_id),
                "agents:balance",
                Some(target.element.trim_start_matches("agents:")),
                Some(
                    &json!({"vendor": vendor, "ok": outcome.is_ok(), "http_status": probe.status}),
                ),
            );
            let _ = tx.commit();
        }
    }

    let items = outcome.map_err(|message| DispatchError::new("AGENT_ERROR", message))?;
    Ok(json!({
        "domain": "agents",
        "action": "balance",
        "element": target.element,
        "vendor": vendor,
        "items": items,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 只对有官方余额接口的厂商查余额_且同主机() {
        assert_eq!(
            balance_endpoint("https://api.deepseek.com/anthropic"),
            Some((
                "deepseek",
                "https://api.deepseek.com/user/balance".to_string()
            ))
        );
        assert_eq!(
            balance_endpoint("https://API.moonshot.ai/v1"),
            Some((
                "moonshot",
                "https://api.moonshot.ai/v1/users/me/balance".to_string()
            ))
        );
        assert_eq!(
            balance_endpoint("https://api.deepseek.com.evil.example/v1"),
            None
        );
        assert_eq!(
            balance_endpoint("https://open.bigmodel.cn/api/anthropic"),
            None
        );
    }

    #[test]
    fn 余额按两家文档的字段取出() {
        let deepseek = r#"{"is_available":true,"balance_infos":[{"currency":"CNY","total_balance":"110.00","granted_balance":"10.00","topped_up_balance":"100.00"}]}"#;
        assert_eq!(
            balance_items("deepseek", deepseek),
            Some(vec![
                json!({"label": "余额", "amount": "110.00", "currency": "CNY"})
            ])
        );
        let moonshot = r#"{"code":0,"data":{"available_balance":49.58,"voucher_balance":46.58,"cash_balance":3.0},"scode":"0x0","status":true}"#;
        let items = balance_items("moonshot", moonshot).unwrap();
        assert_eq!(
            items[0],
            json!({"label": "可用余额", "amount": "49.58", "currency": null})
        );
        assert_eq!(items.len(), 3);
        assert_eq!(balance_items("deepseek", "{}"), None);
    }

    /// 列模型只在接入地址的同一主机下找：Anthropic 协议的 /v1/models 优先，再去掉兼容后缀
    /// 试同一厂商的 OpenAI 协议列表；Codex 地址已带 /v1 时不重复拼。
    #[test]
    fn 列模型的候选地址都在同一主机下() {
        assert_eq!(
            model_list_urls(Kind::Claude, "https://api.moonshot.cn/anthropic/"),
            vec![
                "https://api.moonshot.cn/anthropic/v1/models",
                "https://api.moonshot.cn/anthropic/models",
                "https://api.moonshot.cn/v1/models",
                "https://api.moonshot.cn/models",
            ]
        );
        assert_eq!(
            model_list_urls(Kind::Claude, "https://open.bigmodel.cn/api/anthropic"),
            vec![
                "https://open.bigmodel.cn/api/anthropic/v1/models",
                "https://open.bigmodel.cn/api/anthropic/models",
                "https://open.bigmodel.cn/v1/models",
                "https://open.bigmodel.cn/models",
            ]
        );
        assert_eq!(
            model_list_urls(Kind::Codex, "https://api.deepseek.com/v1"),
            vec!["https://api.deepseek.com/v1/models"]
        );
        assert_eq!(
            model_list_urls(Kind::Codex, "https://api.deepseek.com"),
            vec![
                "https://api.deepseek.com/models",
                "https://api.deepseek.com/v1/models"
            ]
        );
    }

    #[test]
    fn 模型列表按常见形状取出_id() {
        assert_eq!(
            model_ids(
                r#"{"data":[{"id":"kimi-k3","object":"model"},{"id":"kimi-k3"},{"id":" moonshot-v2 "}]}"#
            ),
            Some(vec!["kimi-k3".to_string(), "moonshot-v2".to_string()])
        );
        assert_eq!(
            model_ids(
                r#"{"data":[{"type":"model","id":"claude-sonnet-5","display_name":"Sonnet"}],"has_more":false}"#
            ),
            Some(vec!["claude-sonnet-5".to_string()])
        );
        assert_eq!(
            model_ids(r#"{"models":[{"name":"glm-5.3"}]}"#),
            Some(vec!["glm-5.3".to_string()])
        );
        assert_eq!(
            model_ids(r#"["a","b"]"#),
            Some(vec!["a".to_string(), "b".to_string()])
        );
        assert_eq!(model_ids(r#"{"data":[]}"#), None);
        assert_eq!(model_ids("<html>"), None);
    }

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
        let bearer = headers(Kind::Claude, AuthKind::Token, Some("T"));
        assert!(bearer.iter().any(|h| h == "authorization: Bearer T"));
        assert!(bearer.iter().any(|h| h.starts_with("anthropic-version:")));

        let api_key = headers(Kind::Claude, AuthKind::ApiKey, Some("T"));
        assert!(api_key.iter().any(|h| h == "x-api-key: T"));
        assert!(!api_key.iter().any(|h| h.starts_with("authorization:")));

        // Codex 的 provider 用 env_key，按 Bearer 发
        let codex = headers(Kind::Codex, AuthKind::ApiKey, Some("T"));
        assert!(codex.iter().any(|h| h == "authorization: Bearer T"));

        // 引用了不需要令牌的接入方式：不带任何认证头
        let keyless = headers(Kind::Codex, AuthKind::Token, None);
        assert!(!keyless.iter().any(|h| h.starts_with("authorization:")));
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

    /// 只收编码助手客户端请求的端点：不冒充客户端，如实说明验不了，而不是说「令牌没权限」。
    #[test]
    fn 只收编码助手客户端的端点_说明无法代为检查() {
        let body = r#"{"error":{"message":"Kimi For Coding is currently only available for Coding Agents such as Kimi CLI, Claude Code, Roo Code","type":"access_terminated_error"}}"#;
        let (ok, text) = verdict(Kind::Codex, &probe(403, body));
        assert!(!ok);
        assert!(text.contains("无法代为检查"), "{text}");
        // 普通的 403 照旧
        let (_, text) = verdict(
            Kind::Codex,
            &probe(403, r#"{"error":{"message":"model not enabled"}}"#),
        );
        assert!(text.contains("没有这个权限"), "{text}");
    }

    /// 返回网页（防火墙、Cloudflare 或填成了网站首页）时直说，不当成接口错误去猜。
    #[test]
    fn 返回网页时说明可能被拦截() {
        let (ok, text) = verdict(
            Kind::Claude,
            &probe(
                403,
                "<!DOCTYPE html><html><head><title>Just a moment...</title>",
            ),
        );
        assert!(!ok);
        assert!(text.contains("网页"), "{text}");
        assert!(text.contains("403"), "{text}");
    }

    #[test]
    fn curl_config_转义_引号与反斜杠不破坏结构() {
        assert_eq!(cfg_quote(r#"a"b"#), r#""a\"b""#);
        assert_eq!(cfg_quote(r"a\b"), r#""a\\b""#);
    }
}
