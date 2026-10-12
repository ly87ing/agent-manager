//! Rust 原生 system:refresh-upstream：上游探测宇宙（agents/mcp/skills）的唯一
//! 写探测值命令，及三域 status freshness 回填的只读缓存读取。
//!
//! 语义契约（冻结）：
//! - 探测缓存落 SQLite `upstream_probes` 表（store::UPSTREAM_PROBES_DDL）；只有
//!   refresh-upstream 写（Immediate 事务逐元素 upsert），status 经
//!   `load_probe_cache` 只读连接读取，缺表/缺行一律 freshness unknown 且不建表。
//! - 探测宇宙：enabled 且首选 provider install 命令可解析出 npm 包的 agents
//!   （detect-only 语义不变：可探测但不因此获得收敛动作）+ 各助手实际 MCP 条目与
//!   已保存项中的 npx 包与 uvx `git+repo@branch` 源（每个服务名一份）+ enabled 且有
//!   url 的 skills 集合仓库。
//!   rules/aliases/secrets 恒 not-applicable，不参与。
//! - 探测器带超时（网络 ≤30s）：npm 包 `npm view <pkg> version`；agents 本地版本
//!   沿用 bin --version；MCP npx 本地版本读 npx 缓存
//!   （~/.npm/_npx/*/node_modules/<pkg>/package.json）；skills 集合本地 HEAD
//!   `git rev-parse HEAD`、上游 `git ls-remote <url> HEAD`；uvx git 源上游
//!   git ls-remote、本地 current=null → state unknown（对齐旧语义）。单元素失败
//!   → 该元素 unknown+error 类别，不拖垮整批。
//! - 比较：语义化版本元组可比 → current/behind/ahead；commit 等不可比 → 相同
//!   current、不同 behind；current/upstream 缺失 → unknown。
//! - 缓存不存任何凭据：只写版本/commit 字符串、探测机制名与固定错误类别
//!   （不拼接 URL/命令/环境值，错误类别无需再脱敏）。
//! - refresh-upstream --offline 拒绝（OFFLINE_NETWORK_DENIED）；--only 过滤未知
//!   元素报 USAGE_ERROR；不取全局 apply 锁、不改任何用户配置/策略。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::Result;
use rusqlite::TransactionBehavior;
use serde_json::{json, Value};

use crate::native::{Context, DispatchError, DispatchResult, Request};
use crate::native_agents::{self, AgentDef};
use crate::store;

const NETWORK_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const LOCAL_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// 单路输出有界捕获上限（版本/哈希行远低于此）。
const CAPTURE_LIMIT: usize = 4096;
/// 远端引用清单的捕获上限：大仓库（上百个分支与标签）的 ls-remote 输出可达数百 KB。
const REFS_CAPTURE_LIMIT: usize = 16 * 1024 * 1024;

const PROBE_NPM_VIEW: &str = "npm-view";
const PROBE_GIT_LS_REMOTE: &str = "git-ls-remote";
const PROBE_PYPI_JSON: &str = "pypi-json";
const ERR_PYPI_FAILED: &str = "pypi-failed";

/// 固定错误类别（缓存只存类别，不存底层错误正文/URL/命令）。
const ERR_NPM_VIEW_FAILED: &str = "npm-view-failed";
const ERR_GIT_LS_REMOTE_FAILED: &str = "git-ls-remote-failed";
const ERR_LOCAL_VERSION_MISSING: &str = "local-version-missing";
const ERR_LOCAL_HEAD_MISSING: &str = "local-head-missing";
const ERR_UVX_LOCAL_UNAVAILABLE: &str = "uvx-local-unavailable";
/// 技能来源的跟踪方式解析不出目标（写错，或远端没有匹配的标签 / 分支）。
const ERR_TRACK_UNRESOLVED: &str = "track-unresolved";

// ---------------------------------------------------------------------------
// 缓存读写
// ---------------------------------------------------------------------------

/// 一行探测缓存。字段与 upstream_probes 列一一对应。
pub(crate) struct ProbeRecord {
    pub probe: Option<String>,
    pub state: String,
    pub current: Option<String>,
    pub upstream: Option<String>,
    pub error: Option<String>,
    pub checked_at: String,
}

/// 只读加载全部探测缓存。DB 不存在 → 空；表不存在 → 空（status 零写，
/// 绝不建表）；单行损坏 → 跳过该行，不拖垮其他元素的回填。
pub(crate) fn load_probe_cache(ctx: &Context) -> Result<BTreeMap<String, ProbeRecord>> {
    let Some(conn) = store::open_readonly(ctx)? else {
        return Ok(BTreeMap::new());
    };
    let mut stmt = match conn.prepare(
        "SELECT element_id, probe, state, current, upstream, error, checked_at FROM upstream_probes",
    ) {
        Ok(stmt) => stmt,
        Err(err) => {
            if err.to_string().contains("no such table") {
                return Ok(BTreeMap::new());
            }
            return Err(err.into());
        }
    };
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            ProbeRecord {
                probe: row.get(1)?,
                state: row.get(2)?,
                current: row.get(3)?,
                upstream: row.get(4)?,
                error: row.get(5)?,
                checked_at: row.get(6)?,
            },
        ))
    })?;
    let mut out = BTreeMap::new();
    for row in rows.flatten() {
        out.insert(row.0, row.1);
    }
    Ok(out)
}

fn cache_age_sec(checked_at: &str) -> Option<i64> {
    let parsed = chrono::DateTime::parse_from_rfc3339(checked_at).ok()?;
    let age = chrono::Utc::now()
        .signed_duration_since(parsed.with_timezone(&chrono::Utc))
        .num_seconds();
    Some(age.max(0))
}

/// status 回填：probeable 元素从缓存构造 freshness 块；无记录 → unknown +
/// nulls（current 仍携带本地已观测值）。TTL 不在引擎侧强制，如实输出
/// checked_at 与派生的 cache_age_sec（UI 负责 stale 标注）。
pub(crate) fn freshness_from_cache(
    cache: &BTreeMap<String, ProbeRecord>,
    element_id: &str,
    local_current: Option<&str>,
) -> Value {
    let Some(record) = cache.get(element_id) else {
        return json!({
            "state": "unknown",
            "current": local_current,
            "upstream": Value::Null,
            "checked_at": Value::Null,
            "probe": Value::Null,
            "cache_age_sec": Value::Null,
        });
    };
    json!({
        "state": record.state.as_str(),
        "current": record.current.as_deref().or(local_current),
        "upstream": record.upstream.as_deref(),
        "checked_at": record.checked_at.as_str(),
        "probe": record.probe.as_deref(),
        "cache_age_sec": cache_age_sec(&record.checked_at),
    })
}

// ---------------------------------------------------------------------------
// 探测宇宙分类
// ---------------------------------------------------------------------------

/// 从 `npm install [-g] <pkg>...` 命令提取包名（对齐旧 Python
/// _extract_npm_package_name：scoped 包只有第二个 @ 才是版本分隔）。
fn extract_npm_package_name(command: &str) -> Option<String> {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    if tokens.len() < 3 || tokens[0] != "npm" || tokens[1] != "install" {
        return None;
    }
    for token in &tokens[2..] {
        if token.starts_with('-') {
            continue;
        }
        let name = if let Some(stripped) = token.strip_prefix('@') {
            match stripped.find('@') {
                Some(pos) => &token[..pos + 1],
                None => token,
            }
        } else {
            token.split('@').next().unwrap_or(token)
        };
        if name.is_empty() {
            return None;
        }
        return Some(name.to_string());
    }
    None
}

/// agent 是否在探测宇宙内：enabled 且首选 provider 的 install 命令可解析出
/// npm 包。detect-only（安装来源不可管理）不改变可探测性。
pub(crate) fn agent_probe_package(def: &AgentDef) -> Option<String> {
    if !def.enabled {
        return None;
    }
    // 显式声明优先：官方安装器装的助手没有包名可从命令里推，但 npm 上发的是同一份版本号
    if let Some(explicit) = def.upstream_npm() {
        return Some(explicit.to_string());
    }
    // 旧版本存下的定义没有这个字段：按助手 id 回落到内置定义里的包名（只用于查新版本，不安装）
    if let Some(builtin) = store::builtin_policy()
        .pointer(&format!("/agents/definitions/{}/upstream_npm", def.id))
        .and_then(Value::as_str)
    {
        return Some(builtin.to_string());
    }
    let (install, _) = def.commands.install.as_ref()?;
    extract_npm_package_name(install)
}

pub(crate) enum McpProbeSpec {
    Npx(String),
    UvxGit { repo: String, branch: String },
}

/// MCP 定义分类：npx 的第一个非 flag 参数为 npm 包；uvx 的 `git+repo@branch`
/// 参数为 git 源（对齐旧 Python _classify_mcp_probe）。其余命令不参与探测。
pub(crate) fn classify_mcp_probe(definition: &Value) -> Option<McpProbeSpec> {
    let command = definition.get("command").and_then(Value::as_str)?;
    let args = definition.get("args").and_then(Value::as_array)?;
    match command {
        "npx" => args
            .iter()
            .filter_map(Value::as_str)
            .find(|arg| !arg.starts_with('-'))
            .map(|package| McpProbeSpec::Npx(package.to_string())),
        "uvx" => args.iter().filter_map(Value::as_str).find_map(|arg| {
            let repo_and_branch = arg.strip_prefix("git+")?;
            let (repo, branch) = repo_and_branch.rsplit_once('@')?;
            if repo.is_empty() || branch.is_empty() {
                return None;
            }
            Some(McpProbeSpec::UvxGit {
                repo: repo.to_string(),
                branch: branch.to_string(),
            })
        }),
        _ => None,
    }
}

enum ProbeKind {
    AgentNpm {
        bin: String,
        package: String,
    },
    McpNpx {
        package: String,
    },
    McpUvxGit {
        repo: String,
        branch: String,
    },
    /// 技能与 MCP 依赖的程序（代管、托管）：本地版本由程序域读出，上游问 npm / PyPI
    Program {
        registry: &'static str,
        package: String,
        current: Option<String>,
    },
    SkillGit {
        repo_dir: PathBuf,
        url: String,
        /// 来源定义里的跟踪方式（原样）；缺省为自动
        track: Option<Value>,
    },
}

struct ProbeTarget {
    id: String,
    kind: ProbeKind,
}

fn collect_targets(ctx: &Context) -> Result<Vec<ProbeTarget>> {
    let mut targets = Vec::new();

    // MCP：各助手实际条目与已保存项里的 npx 包 / uvx git 源，每个服务名取一份
    // （选取规则与 mcp:status 的 freshness 一致）。
    let mcp = crate::native_mcp::inventory::load(ctx)?;
    let mut servers: Vec<String> = mcp
        .scans
        .iter()
        .flat_map(|scan| scan.entries.iter().map(|e| e.name.clone()))
        .chain(mcp.saved.iter().map(|row| row.server.clone()))
        .collect();
    servers.sort();
    servers.dedup();
    for server in &servers {
        let Some(def) = crate::native_mcp::status::probe_definition(&mcp, server) else {
            continue;
        };
        let kind = match classify_mcp_probe(&def) {
            Some(McpProbeSpec::Npx(package)) => ProbeKind::McpNpx { package },
            Some(McpProbeSpec::UvxGit { repo, branch }) => ProbeKind::McpUvxGit { repo, branch },
            None => continue,
        };
        targets.push(ProbeTarget {
            id: format!("mcp:{}", server),
            kind,
        });
    }

    // 程序：代管与托管的 npm、PyPI 包（经 npx / uvx 运行的沿用上面 MCP 的探测）
    if let Ok(programs) = crate::native_programs::status::status(ctx, false) {
        for element in programs
            .get("elements")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let mode = element
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !matches!(mode, "delegated" | "managed") {
                continue;
            }
            let (Some(kind), Some(name), Some(id)) = (
                element.get("kind").and_then(Value::as_str),
                element.get("name").and_then(Value::as_str),
                element.get("id").and_then(Value::as_str),
            ) else {
                continue;
            };
            let registry = match kind {
                "npm" => "npm",
                "pypi" => "pypi",
                _ => continue,
            };
            targets.push(ProbeTarget {
                id: id.to_string(),
                kind: ProbeKind::Program {
                    registry,
                    package: name.to_string(),
                    current: element
                        .get("version")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                },
            });
        }
    }

    let policy = store::effective_policy(ctx)?;
    // 旧库固化、从未生效的内置集合按推荐处理，推荐不参与上游检查
    let hidden_skills = crate::native_skills::hidden_builtin_collections(ctx, &policy)?;
    if let Some(repos) = policy
        .get("skills")
        .and_then(|v| v.get("repositories"))
        .and_then(Value::as_object)
    {
        for (name, raw) in repos {
            if raw.get("_delete") == Some(&Value::Bool(true)) || hidden_skills.contains(name) {
                continue;
            }
            let enabled = policy
                .get("skills")
                .and_then(|v| v.get(name))
                .and_then(|v| v.get("enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let Some(url) = raw.get("url").and_then(Value::as_str) else {
                continue;
            };
            if !enabled || url.is_empty() {
                continue;
            }
            targets.push(ProbeTarget {
                id: format!("skill:{}", name),
                kind: ProbeKind::SkillGit {
                    repo_dir: crate::native_skills::repo_base_dir(ctx).join(name),
                    url: url.to_string(),
                    track: raw.get("track").cloned(),
                },
            });
        }
    }

    targets.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(targets)
}

// ---------------------------------------------------------------------------
// 探测器（有界捕获 + 超时终止进程组，失败 → None，由上层归 unknown+error）
// ---------------------------------------------------------------------------

/// 以参数数组直接执行（不经 shell），exit 0 → Some(stdout)；超时/非 0/启动
/// 失败 → None。stdin 关闭，HOME 注入 ctx.home_dir（探测不继承调用方 HOME）。
fn run_capture(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    ctx: &Context,
    timeout: Duration,
) -> Option<String> {
    let mut command = crate::process::command(program);
    command.args(args).env("HOME", &ctx.home_dir);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let (code, stdout, _) = capture_exit(command, timeout)?;
    if code != 0 {
        return None;
    }
    String::from_utf8(stdout).ok()
}

/// 执行调用方配好参数与环境的命令：stdin 关闭、独立进程组、输出有界。
/// 正常退出且管道 EOF → Some((退出码, stdout, stderr 前 256 字节))；
/// 进程或管道超时、被信号终止或启动失败 → None；超时时终止进程组。
pub(crate) fn capture_exit(command: Command, timeout: Duration) -> Option<(i32, Vec<u8>, Vec<u8>)> {
    capture_exit_limited(command, timeout, CAPTURE_LIMIT)
}

/// 同 `capture_exit`，stdout 上限由调用方给出。
fn capture_exit_limited(
    mut command: Command,
    timeout: Duration,
    limit: usize,
) -> Option<(i32, Vec<u8>, Vec<u8>)> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut output = crate::process::capture_group(&mut command, timeout, limit.max(256)).ok()?;
    output.stdout.truncate(limit);
    output.stderr.truncate(256);
    Some((output.status.code()?, output.stdout, output.stderr))
}

/// npm 包名白名单校验（防策略值被拼成 flag）；不合法 → 探测失败类别。
fn valid_npm_package(package: &str) -> bool {
    !package.is_empty()
        && !package.starts_with('-')
        && package
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '/' | '.' | '_' | '-'))
}

/// Resolve tools through the same user PATH/PATHEXT rules as program management.
/// Pass that PATH to the child as well so npm can find its Node runtime.
fn run_user_tool(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    ctx: &Context,
    timeout: Duration,
) -> Option<String> {
    let path = crate::native_programs::locate::user_path(ctx, false);
    let executable = crate::native_programs::locate::find_all(&path, program)
        .into_iter()
        .next()?;
    let mut command = crate::process::command(executable);
    command
        .args(args)
        .env("HOME", &ctx.home_dir)
        .env("PATH", std::env::join_paths(&path.dirs).ok()?);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let (code, stdout, _) = capture_exit(command, timeout)?;
    (code == 0)
        .then(|| String::from_utf8(stdout).ok())
        .flatten()
}

fn npm_view(ctx: &Context, package: &str) -> Option<String> {
    if !valid_npm_package(package) {
        return None;
    }
    let out = run_user_tool(
        "npm",
        &["view", package, "version"],
        None,
        ctx,
        NETWORK_PROBE_TIMEOUT,
    )?;
    out.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// PyPI 上一个包的最新版本（JSON 接口）。
fn pypi_latest(ctx: &Context, package: &str) -> Option<String> {
    if package.is_empty()
        || !package
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return None;
    }
    let url = format!("https://pypi.org/pypi/{package}/json");
    let out = run_user_tool(
        "curl",
        &["-fsSL", "--max-time", "25", &url],
        None,
        ctx,
        NETWORK_PROBE_TIMEOUT,
    )?;
    let value: Value = serde_json::from_str(&out).ok()?;
    value
        .pointer("/info/version")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// git ls-remote 取指定 ref 的第一行第一列（commit hash）。repo/ref 校验非空
/// 且不以 '-' 开头（防被解释为 flag）。
fn git_ls_remote(ctx: &Context, repo: &str, git_ref: &str) -> Option<String> {
    if repo.is_empty() || git_ref.is_empty() || repo.starts_with('-') || git_ref.starts_with('-') {
        return None;
    }
    let out = run_capture(
        "git",
        &["ls-remote", repo, git_ref],
        None,
        ctx,
        NETWORK_PROBE_TIMEOUT,
    )?;
    let first = out.lines().map(str::trim).find(|line| !line.is_empty())?;
    let hash = first.split_whitespace().next()?;
    if hash.is_empty() {
        return None;
    }
    Some(hash.to_string())
}

/// 远端的分支、标签与默认分支：`git ls-remote --symref <url> HEAD refs/heads/* refs/tags/*`。
/// 只读网络；不交互（凭据失效时直接失败）；错误只给固定类别，不回显 URL 与命令输出。
pub(crate) fn git_remote_refs(
    ctx: &Context,
    url: &str,
) -> std::result::Result<crate::native_skills::track::RemoteRefs, &'static str> {
    if url.is_empty() || url.starts_with('-') {
        return Err(ERR_GIT_LS_REMOTE_FAILED);
    }
    let mut command = crate::platform::git_command();
    command
        .args([
            "ls-remote",
            "--symref",
            "--",
            url,
            "HEAD",
            "refs/heads/*",
            "refs/tags/*",
        ])
        .env("HOME", &ctx.home_dir)
        .env("GIT_TERMINAL_PROMPT", "0");
    let (code, stdout, _) =
        capture_exit_limited(command, NETWORK_PROBE_TIMEOUT * 2, REFS_CAPTURE_LIMIT)
            .ok_or(ERR_GIT_LS_REMOTE_FAILED)?;
    if code != 0 || stdout.len() >= REFS_CAPTURE_LIMIT {
        return Err(ERR_GIT_LS_REMOTE_FAILED);
    }
    let text = String::from_utf8(stdout).map_err(|_| ERR_GIT_LS_REMOTE_FAILED)?;
    Ok(crate::native_skills::track::RemoteRefs::parse(&text))
}

fn git_local_head(ctx: &Context, repo_dir: &Path) -> Option<String> {
    if !repo_dir.is_dir() {
        return None;
    }
    let out = run_capture(
        "git",
        &["rev-parse", "HEAD"],
        Some(repo_dir),
        ctx,
        LOCAL_PROBE_TIMEOUT,
    )?;
    let first = out.lines().map(str::trim).find(|line| !line.is_empty())?;
    if first.is_empty() {
        return None;
    }
    Some(first.to_string())
}

/// 把 `chrome-devtools-mcp@1.9.0` 拆成包名和钉死的版本；作用域包 `@scope/name` 的
/// 首个 `@` 不是版本分隔符。没写版本就是"跟最新"。
pub(crate) fn split_package_spec(spec: &str) -> (&str, Option<&str>) {
    let search_from = usize::from(spec.starts_with('@'));
    match spec[search_from..].find('@') {
        Some(offset) => {
            let at = search_from + offset;
            let version = spec[at + 1..].trim();
            if version.is_empty() {
                (&spec[..at], None)
            } else {
                (&spec[..at], Some(version))
            }
        }
        None => (spec, None),
    }
}

/// 本机 npx 缓存里这个包的最高版本（~/.npm/_npx/*/node_modules/<pkg>/package.json），
/// 纯本地文件读取，无网络；缺失 → None。
///
/// npx 按调用参数哈希分目录，同一个包可能在 `~/.npm/_npx` 下留着好几份不同版本
/// （真机上见过 8 份）。原先按目录名字典序取第一个能读到的，报出来的版本跟实际会跑
/// 哪个毫无关系——真机上本地最高已有 1.8.0，却报成 1.2.0。取最高版本：它代表
/// "本机已经拉到过的最新的那份"，再低就确实没拉到过。
fn npx_cached_version(home_dir: &Path, package: &str) -> Option<String> {
    if !valid_npm_package(package) {
        return None;
    }
    // npm 缓存默认位置（npm 文档 config#cache）：Posix 为 ~/.npm，Windows 为 %LocalAppData%\npm-cache；
    // Windows 按主目录下的 AppData\Local 取（与 %LocalAppData% 的默认值一致，也便于隔离测试）
    let cache_base = if cfg!(windows) {
        home_dir
            .join("AppData")
            .join("Local")
            .join("npm-cache")
            .join("_npx")
    } else {
        home_dir.join(".npm").join("_npx")
    };
    let entries: Vec<PathBuf> = std::fs::read_dir(&cache_base)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    let mut rel = PathBuf::from("node_modules");
    for segment in package.split('/') {
        rel.push(segment);
    }
    rel.push("package.json");
    let mut best: Option<String> = None;
    for entry in entries {
        let package_json = entry.join(&rel);
        let Ok(raw) = std::fs::read_to_string(&package_json) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let Some(version) = parsed.get("version").and_then(Value::as_str) else {
            continue;
        };
        let version = version.trim();
        if version.is_empty() {
            continue;
        }
        let better = match &best {
            None => true,
            Some(current) => match (version_tuple(version), version_tuple(current)) {
                (Some(a), Some(b)) => a > b,
                // 版本号排不出先后时不换，保持先读到的那个
                _ => false,
            },
        };
        if better {
            best = Some(version.to_string());
        }
    }
    best
}

// ---------------------------------------------------------------------------
// 比较（对齐旧 Python _compare_freshness）
// ---------------------------------------------------------------------------

fn version_tuple(text: &str) -> Option<Vec<u64>> {
    let re = regex::Regex::new(r"\d+(?:\.\d+)+").expect("静态正则合法");
    let token = re.find(text)?.as_str();
    let mut parts = Vec::new();
    for part in token.split('.') {
        // 位宽保护：超长数字段按不可比处理（对齐 unorderable → behind）
        if part.len() > 18 {
            return None;
        }
        parts.push(part.parse::<u64>().ok()?);
    }
    Some(parts)
}

fn compare_freshness(current: Option<&str>, upstream: Option<&str>) -> &'static str {
    let (Some(current), Some(upstream)) = (current, upstream) else {
        return "unknown";
    };
    if current == upstream {
        return "current";
    }
    if let (Some(mut current_key), Some(mut upstream_key)) =
        (version_tuple(current), version_tuple(upstream))
    {
        let width = current_key.len().max(upstream_key.len());
        current_key.resize(width, 0);
        upstream_key.resize(width, 0);
        if current_key == upstream_key {
            return "current";
        }
        return if current_key > upstream_key {
            "ahead"
        } else {
            "behind"
        };
    }
    // 不可比值（如 commit hash）：任何差异按 behind；只有版本可有序时才断言 ahead。
    "behind"
}

// ---------------------------------------------------------------------------
// 单元素探测
// ---------------------------------------------------------------------------

fn probe_target(ctx: &Context, target: &ProbeTarget) -> ProbeRecord {
    let (probe, mut state, current, upstream, error): (
        &'static str,
        &'static str,
        Option<String>,
        Option<String>,
        Option<&'static str>,
    ) = match &target.kind {
        ProbeKind::AgentNpm { bin, package } => {
            let current = native_agents::resolve_bin(&ctx.home_dir, bin)
                .as_deref()
                .and_then(native_agents::probe_version);
            match npm_view(ctx, package) {
                None => (
                    PROBE_NPM_VIEW,
                    "unknown",
                    current,
                    None,
                    Some(ERR_NPM_VIEW_FAILED),
                ),
                Some(upstream) => {
                    let state = compare_freshness(current.as_deref(), Some(&upstream));
                    let error = if state == "unknown" {
                        Some(ERR_LOCAL_VERSION_MISSING)
                    } else {
                        None
                    };
                    (PROBE_NPM_VIEW, state, current, Some(upstream), error)
                }
            }
        }
        ProbeKind::McpNpx { package } => {
            let (name, pinned) = split_package_spec(package);
            // 定义里钉死了版本，跑的就是它；没钉死才去看本机缓存拉到过哪些。
            let current = match pinned {
                Some(version) => Some(version.to_string()),
                None => npx_cached_version(&ctx.home_dir, name),
            };
            // 上游一律问 latest：`npm view foo@1.2.0 version` 只会把 1.2.0 原样回来，
            // 拿它当上游就永远比不出落后。
            match npm_view(ctx, name) {
                None => (
                    PROBE_NPM_VIEW,
                    "unknown",
                    current,
                    None,
                    Some(ERR_NPM_VIEW_FAILED),
                ),
                Some(upstream) => {
                    let state = compare_freshness(current.as_deref(), Some(&upstream));
                    let error = if state == "unknown" {
                        Some(ERR_LOCAL_VERSION_MISSING)
                    } else {
                        None
                    };
                    (PROBE_NPM_VIEW, state, current, Some(upstream), error)
                }
            }
        }
        ProbeKind::Program {
            registry,
            package,
            current,
        } => {
            let (probe, upstream, err) = match *registry {
                "npm" => (PROBE_NPM_VIEW, npm_view(ctx, package), ERR_NPM_VIEW_FAILED),
                _ => (PROBE_PYPI_JSON, pypi_latest(ctx, package), ERR_PYPI_FAILED),
            };
            match upstream {
                None => (probe, "unknown", current.clone(), None, Some(err)),
                Some(upstream) => {
                    let state = compare_freshness(current.as_deref(), Some(&upstream));
                    let error = (state == "unknown").then_some(ERR_LOCAL_VERSION_MISSING);
                    (probe, state, current.clone(), Some(upstream), error)
                }
            }
        }
        ProbeKind::McpUvxGit { repo, branch } => match git_ls_remote(ctx, repo, branch) {
            None => (
                PROBE_GIT_LS_REMOTE,
                "unknown",
                None,
                None,
                Some(ERR_GIT_LS_REMOTE_FAILED),
            ),
            // uvx 本地缓存 commit 不可读（对齐旧语义）：上游值仍入库供展示。
            Some(upstream) => (
                PROBE_GIT_LS_REMOTE,
                "unknown",
                None,
                Some(upstream),
                Some(ERR_UVX_LOCAL_UNAVAILABLE),
            ),
        },
        ProbeKind::SkillGit {
            repo_dir,
            url,
            track,
        } => {
            // 本地：镜像所在提交与名字（检出时记下的标签 / 分支名；旧镜像退回 git rev-parse）
            let local = crate::native_skills::versioning::local_version(repo_dir);
            let local_commit = local
                .get("commit")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| git_local_head(ctx, repo_dir));
            let track = crate::native_skills::track::Track::parse(track.as_ref());
            match (crate::native_upstream::git_remote_refs(ctx, url), track) {
                (Ok(refs), Ok(track)) => {
                    let current = local_commit.as_ref().map(|commit| {
                        match local.get("kind").and_then(Value::as_str) {
                            Some("tag") => local["label"].as_str().unwrap_or_default().to_string(),
                            _ => refs
                                .tag_at(commit)
                                .unwrap_or_else(|| commit.chars().take(7).collect()),
                        }
                    });
                    match crate::native_skills::track::resolve(&track, &refs) {
                        Ok(target) => {
                            let state = match &local_commit {
                                None => "unknown",
                                Some(commit) if *commit == target.commit => "current",
                                Some(_) => "behind",
                            };
                            let error = local_commit.is_none().then_some(ERR_LOCAL_HEAD_MISSING);
                            // 标签显示标签名；分支与提交显示前 7 位提交，与本地一侧同一写法
                            let upstream = if target.kind == "tag" {
                                target.name.clone()
                            } else {
                                target.commit.chars().take(7).collect()
                            };
                            (PROBE_GIT_LS_REMOTE, state, current, Some(upstream), error)
                        }
                        Err(_) => (
                            PROBE_GIT_LS_REMOTE,
                            "unknown",
                            current,
                            None,
                            Some(ERR_TRACK_UNRESOLVED),
                        ),
                    }
                }
                (Ok(_), Err(_)) => (
                    PROBE_GIT_LS_REMOTE,
                    "unknown",
                    local_commit,
                    None,
                    Some(ERR_TRACK_UNRESOLVED),
                ),
                (Err(_), _) => (
                    PROBE_GIT_LS_REMOTE,
                    "unknown",
                    local_commit,
                    None,
                    Some(ERR_GIT_LS_REMOTE_FAILED),
                ),
            }
        }
    };
    // 防御：比较器只产出四态；异常值统一归 unknown。
    if !matches!(state, "current" | "behind" | "ahead" | "unknown") {
        state = "unknown";
    }
    ProbeRecord {
        probe: Some(probe.to_string()),
        state: state.to_string(),
        current,
        upstream,
        error: error.map(str::to_string),
        checked_at: String::new(),
    }
}

// ---------------------------------------------------------------------------
// handle
// ---------------------------------------------------------------------------

/// 严格参数：只接受可选的单个 --only <element-id>；--offline 固定拒绝
/// （OFFLINE_NETWORK_DENIED）；位置参数、未知 flag、重复 --only、缺失/类 flag
/// 取值均 USAGE_ERROR，错误消息固定，不回显取值。
fn parse_args(request: &Request) -> std::result::Result<Option<String>, DispatchError> {
    let mut only: Option<String> = None;
    let mut index = 0;
    while index < request.args.len() {
        match request.args[index].as_str() {
            "--offline" => {
                return Err(DispatchError::new(
                    "OFFLINE_NETWORK_DENIED",
                    "system:refresh-upstream 需要网络探测，offline 模式下拒绝执行",
                ));
            }
            "--only" => {
                if only.is_some() {
                    return Err(DispatchError::new(
                        "USAGE_ERROR",
                        "system:refresh-upstream 仅接受可选的单个 --only <元素id>",
                    ));
                }
                let Some(value) = request.args.get(index + 1) else {
                    return Err(DispatchError::new(
                        "USAGE_ERROR",
                        "system:refresh-upstream 仅接受可选的单个 --only <元素id>",
                    ));
                };
                if value.starts_with("--") || value.is_empty() {
                    return Err(DispatchError::new(
                        "USAGE_ERROR",
                        "system:refresh-upstream 仅接受可选的单个 --only <元素id>",
                    ));
                }
                only = Some(value.clone());
                index += 2;
            }
            _ => {
                return Err(DispatchError::new(
                    "USAGE_ERROR",
                    "system:refresh-upstream 仅接受可选的单个 --only <元素id>",
                ));
            }
        }
    }
    Ok(only)
}

pub(crate) fn handle(ctx: &Context, request: &Request) -> DispatchResult {
    let only = parse_args(request)?;
    let targets = collect_targets(ctx)?;
    let selected: Vec<ProbeTarget> = match &only {
        Some(id) => {
            let Some(target) = targets.into_iter().find(|target| &target.id == id) else {
                return Err(DispatchError::new(
                    "USAGE_ERROR",
                    "system:refresh-upstream 的 --only 元素不在探测范围内",
                ));
            };
            vec![target]
        }
        None => targets,
    };

    // 先全部探测（可能耗时，不持有任何写连接/锁），再一次 Immediate 事务落库。
    let checked_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut records = Vec::with_capacity(selected.len());
    for target in &selected {
        let mut record = probe_target(ctx, target);
        record.checked_at = checked_at.clone();
        records.push((target.id.clone(), record));
    }

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    store::ensure_upstream_probes(&tx)?;
    for (id, record) in &records {
        tx.execute(
            "INSERT INTO upstream_probes (element_id, probe, state, current, upstream, error, checked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(element_id) DO UPDATE SET
                probe=excluded.probe, state=excluded.state, current=excluded.current,
                upstream=excluded.upstream, error=excluded.error, checked_at=excluded.checked_at",
            rusqlite::params![
                id,
                record.probe.as_deref(),
                record.state,
                record.current.as_deref(),
                record.upstream.as_deref(),
                record.error.as_deref(),
                record.checked_at,
            ],
        )?;
    }
    tx.commit()?;

    let mut probed = 0u64;
    let mut counts: BTreeMap<&str, u64> = BTreeMap::new();
    let mut elements = Vec::with_capacity(records.len());
    for (id, record) in &records {
        probed += 1;
        *counts.entry(record.state.as_str()).or_insert(0) += 1;
        elements.push(json!({
            "id": id,
            "probe": record.probe.as_deref(),
            "state": record.state.as_str(),
            "current": record.current.as_deref(),
            "upstream": record.upstream.as_deref(),
            "error": record.error.as_deref(),
        }));
    }

    let data = json!({
        "domain": "system",
        "verb": "refresh-upstream",
        "cache_file": Value::Null,
        "elements": elements,
        "summary": {
            "probed": probed,
            "current": counts.get("current").copied().unwrap_or(0),
            "behind": counts.get("behind").copied().unwrap_or(0),
            "ahead": counts.get("ahead").copied().unwrap_or(0),
            "unknown": counts.get("unknown").copied().unwrap_or(0),
        },
    });
    Ok((data, vec![]))
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use crate::process::capture_test_support as fixture;
    use std::time::Instant;

    fn command(root: &Path, mode: &str) -> Command {
        let mut command = fixture::command(root, mode);
        command
            .env("HOME", root)
            .env("USERPROFILE", root)
            .current_dir(root);
        command
    }

    fn inherited_pipe_deadline(stream: &str, timeout: Duration) {
        let root = fixture::root(&format!("upstream-{stream}"));
        let start = Instant::now();
        let result = capture_exit(command(&root, stream), timeout);
        let elapsed = start.elapsed();
        let ready = root.join("ready").exists();
        let parent_exited = root.join("parent-exited").exists();
        std::thread::sleep(Duration::from_secs(6).saturating_sub(start.elapsed()));
        let survived = root.join("late").exists();
        std::fs::remove_dir_all(root).unwrap();
        assert!(ready, "The pipe-holding descendant never started");
        assert!(
            parent_exited,
            "The parent did not exit before its pipe holder"
        );
        assert!(
            result.is_none(),
            "Incomplete capture reported success: {result:?}"
        );
        assert!(
            elapsed < timeout + Duration::from_secs(1),
            "Capture exceeded its shared deadline: {elapsed:?}"
        );
        assert!(
            !survived,
            "The timed-out capture left its descendant running"
        );
    }

    #[test]
    fn capture_times_out_after_parent_exits_with_inherited_stdout() {
        inherited_pipe_deadline("stdout", Duration::from_secs(2));
    }

    #[test]
    fn capture_times_out_after_parent_exits_with_inherited_stderr() {
        inherited_pipe_deadline("stderr", Duration::from_secs(2));
    }

    #[test]
    fn capture_uses_one_deadline_for_parent_and_reader_completion() {
        inherited_pipe_deadline("delayed-stdout", Duration::from_secs(3));
    }

    #[test]
    fn capture_preserves_nonzero_exit_and_independent_stream_limits() {
        let root = fixture::root("upstream-output");
        let (code, stdout, stderr) =
            capture_exit_limited(command(&root, "output"), Duration::from_secs(10), 4).unwrap();
        assert_eq!(code, 7);
        assert_eq!(stdout.len(), 4);
        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            (0..10)
                .map(|i| format!("diagnostic-{i}\n"))
                .collect::<String>()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn capture_drains_both_streams_past_retention_limits() {
        let root = fixture::root("upstream-large-output");
        let (code, stdout, stderr) =
            capture_exit(command(&root, "large-output"), Duration::from_secs(10)).unwrap();
        assert_eq!(code, 0);
        assert_eq!(stdout.len(), CAPTURE_LIMIT);
        assert!(stdout.ends_with(b"oooo"));
        assert_eq!(stderr, vec![b'e'; 256]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
