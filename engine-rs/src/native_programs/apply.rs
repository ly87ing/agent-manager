//! `programs:preview` / `programs:apply`：安装、升级、换版本、回退、纳入、删除
//! （docs/PROGRAM-LIFECYCLE.md 第 7–9 节）。预览与执行共用一份计划：预览只算不做，执行照计划做。
//!
//! 两种管理方式：
//! - 代管（npm 全局、uv tool）：程序留在原处，本工具调用原包管理器；改动前把整个包目录与它的
//!   命令链接**移到**备份目录（同分区改名，零网络），失败或回退时原样移回。
//! - 托管：装在数据目录 `programs/<类别>/<名>/<版本>/`，`current` 指向当前版本，`programs/bin/<命令>`
//!   经 `current` 指到真实命令；用户选定的入口目录（已在 PATH 上）里放登记过的链接。
//!
//! 安装脚本默认不执行（与 pnpm 10、Bun、Deno、npm 12 一致），用户对某个程序允许后才执行。

use crate::platform::Canon;
use anyhow::{anyhow, bail, Context as _, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::native::{Context, Request};
use crate::store;

const META_FILE: &str = ".agent-manager-program.json";
const BACKUP_META: &str = "backup.json";
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
/// 同一程序保留的回退备份数；更早的在下一次改动时清掉。
const KEEP_BACKUPS: usize = 3;

fn programs_root(ctx: &Context) -> PathBuf {
    ctx.data_dir.join("programs")
}

/// 程序标识 → 目录名：`npm:@wecom/cli` → `npm/@wecom__cli`。
fn program_dir(ctx: &Context, id: &str) -> Result<PathBuf> {
    let (scheme, name) = id
        .split_once(':')
        .ok_or_else(|| anyhow!("程序标识格式不对：{id}"))?;
    if !matches!(scheme, "npm" | "pypi" | "git") {
        bail!("{id} 不能由本工具安装（只支持 npm、PyPI 与 git 源）");
    }
    let slug: String = name
        .chars()
        .map(|c| if c == '/' { '~' } else { c })
        .collect();
    if slug.is_empty()
        || slug.contains("..")
        || !slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '~' | '.' | '-' | '_'))
    {
        bail!("程序名不合法：{name}");
    }
    Ok(programs_root(ctx).join(scheme).join(slug))
}

fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", crate::platform::slash(&rest)),
        Err(_) => path.to_string_lossy().to_string(),
    }
}

fn expand(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(path),
    }
}

fn valid_version(version: &str) -> bool {
    // Keep one portable filename contract, including case-insensitive filesystems
    // and Windows device names / trailing-dot normalization.
    let base = version
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            base.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9')
            })
        });
    !version.is_empty()
        && !version.eq_ignore_ascii_case("current")
        && !version.starts_with('.')
        && !version.ends_with('.')
        && !reserved
        && version.len() <= 64
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
}

// ---------------------------------------------------------------------------
// 子进程
// ---------------------------------------------------------------------------

struct Output {
    ok: bool,
    stdout: String,
    tail: String,
}

fn run(mut command: Command, timeout: Duration) -> Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output =
        crate::process::capture_group(&mut command, timeout, 4 * 1024 * 1024).map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                anyhow!("命令超时（{} 秒）", timeout.as_secs())
            } else {
                anyhow!(error).context("启动命令失败")
            }
        })?;
    let stdout = String::from_utf8(output.stdout).unwrap_or_default();
    let stderr = String::from_utf8(output.stderr).unwrap_or_default();
    let combined = format!("{stdout}\n{stderr}");
    let tail: String = combined
        .lines()
        .filter(|l| !l.trim().is_empty())
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Output {
        ok: output.status.success(),
        stdout,
        tail: crate::sanitize::sanitize_diagnostic_message(&tail),
    })
}

fn which(bin: &str, user_path: &super::locate::UserPath) -> Option<PathBuf> {
    super::locate::find_all(user_path, bin).into_iter().next()
}

/// npm 上某个包的最新版本（联网）。
fn npm_latest(npm: &Path, package: &str) -> Result<String> {
    let mut cmd = crate::process::command(npm);
    cmd.args(["view", package, "version"]);
    let out = run(cmd, Duration::from_secs(60))?;
    let version = out
        .stdout
        .trim()
        .lines()
        .last()
        .unwrap_or_default()
        .trim()
        .to_string();
    if !out.ok || !valid_version(&version) {
        bail!("查不到 {package} 的最新版本：{}", out.tail);
    }
    Ok(version)
}

/// PyPI 上某个包的最新版本（联网）。
fn pypi_latest(package: &str, user_path: &super::locate::UserPath) -> Result<String> {
    let curl = which("curl", user_path).ok_or_else(|| anyhow!("PATH 上没有 curl"))?;
    let mut cmd = crate::process::command(curl);
    cmd.args([
        "-fsSL",
        "--max-time",
        "30",
        &format!("https://pypi.org/pypi/{package}/json"),
    ]);
    let out = run(cmd, Duration::from_secs(40))?;
    let value: Value =
        serde_json::from_str(&out.stdout).map_err(|_| anyhow!("查不到 {package} 的最新版本"))?;
    let version = value
        .pointer("/info/version")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if !out.ok || !valid_version(&version) {
        bail!("查不到 {package} 的最新版本");
    }
    Ok(version)
}

// ---------------------------------------------------------------------------
// 库：入口目录、入口链接登记、允许安装脚本
// ---------------------------------------------------------------------------

fn update_programs_policy(
    ctx: &Context,
    req: &Request,
    what: &str,
    f: impl FnOnce(&mut Map<String, Value>),
) -> Result<()> {
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut policy = store::policy_from_connection(&tx)?;
    let root = policy
        .as_object_mut()
        .ok_or_else(|| anyhow!("策略不是对象"))?;
    let programs = root.entry("programs").or_insert_with(|| json!({}));
    if !programs.is_object() {
        *programs = json!({});
    }
    f(programs.as_object_mut().expect("刚确保是对象"));
    store::write_policy(&tx, &policy)?;
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "programs:apply",
        Some(what),
        Some(&json!({"what": what})),
    )?;
    tx.commit()?;
    Ok(())
}

fn record(ctx: &Context, req: &Request, target: &str, detail: Value) -> Result<()> {
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "programs:apply",
        Some(target),
        Some(&detail),
    )?;
    tx.commit()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 计划
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Action {
    Install,
    Upgrade,
    Switch,
    Restore,
    Remove,
    Adopt,
    Unadopt,
    Link,
    Unlink,
}

impl Action {
    fn parse(raw: &str) -> Result<Self> {
        Ok(match raw {
            "install" => Self::Install,
            "upgrade" => Self::Upgrade,
            "switch" => Self::Switch,
            "restore" => Self::Restore,
            "remove" => Self::Remove,
            "adopt" => Self::Adopt,
            "unadopt" => Self::Unadopt,
            "link" => Self::Link,
            "unlink" => Self::Unlink,
            other => bail!("不支持的动作 '{other}'"),
        })
    }
}

struct Args {
    program: String,
    action: Action,
    version: Option<String>,
    backup: Option<String>,
    entry_dir: Option<String>,
    allow_scripts: bool,
}

fn parse_args(req: &Request) -> Result<Args> {
    let known = [
        "--program",
        "--action",
        "--version",
        "--backup",
        "--entry-dir",
        "--allow-scripts",
    ];
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--json" {
            continue;
        }
        if !known.contains(&arg.as_str()) {
            bail!("不支持参数 '{arg}'");
        }
        if iter.next().is_none() {
            bail!("参数 {arg} 缺少值");
        }
    }
    let program = req
        .value("--program")
        .ok_or_else(|| anyhow!("需要 --program <程序标识>"))?
        .to_string();
    let action = Action::parse(
        req.value("--action")
            .ok_or_else(|| anyhow!("需要 --action"))?,
    )?;
    let version = req.value("--version").map(str::to_string);
    if let Some(v) = &version {
        if v != "latest" && !valid_version(v) {
            bail!("版本号不合法：{v}");
        }
    }
    let backup = req.value("--backup").map(str::to_string);
    if let Some(b) = &backup {
        if b.is_empty()
            || !b
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        {
            bail!("备份编号不合法");
        }
    }
    let allow_scripts = match req.value("--allow-scripts") {
        None | Some("false") => false,
        Some("true") => true,
        Some(other) => bail!("--allow-scripts 只能是 true 或 false，不是 {other}"),
    };
    Ok(Args {
        program,
        action,
        version,
        backup,
        entry_dir: req.value("--entry-dir").map(str::to_string),
        allow_scripts,
    })
}

/// 计划里的一步：给人看的说明，以及执行时要做的事。
#[derive(Clone, Debug)]
enum Step {
    /// 把 from 整个移到 to（同分区改名）
    Move { from: PathBuf, to: PathBuf },
    /// 执行命令（程序、参数、额外环境变量）
    Exec {
        program: PathBuf,
        args: Vec<String>,
        env: Vec<(String, String)>,
        cwd: Option<PathBuf>,
    },
    /// 建链接 path → target（path 已存在时必须是本工具登记过的链接）
    Link {
        path: PathBuf,
        target: PathBuf,
        register: bool,
    },
    /// 删除链接（只删指向本工具目录的、登记过的）
    Unlink { path: PathBuf },
    /// 删除本工具目录下的一个目录
    RemoveDir { path: PathBuf },
    /// 把 current 指到某个版本目录
    SetCurrent {
        program_dir: PathBuf,
        version: String,
    },
    /// 写版本元数据（装好、校验过后最后一步）
    WriteMeta { dir: PathBuf, meta: Value },
    /// 写备份说明
    WriteBackupMeta { dir: PathBuf, meta: Value },
    /// 保存入口目录选择
    SaveEntryDir { dir: String },
}

pub struct Plan {
    program: String,
    action: Action,
    target_version: Option<String>,
    steps: Vec<Step>,
    /// 执行后要核对的：命令 → 应该被找到的位置（入口链接或原位置）
    verify_bins: Vec<(String, PathBuf)>,
    /// 代管升级：核对包目录里的版本
    verify_version: Option<(PathBuf, String)>,
    /// 失败时要按相反顺序撤回的移动（执行时记录）
    notes: Vec<String>,
    scripts_ignored: bool,
    /// 备份目录（代管改动与纳入）
    backup_dir: Option<PathBuf>,
    /// 自己动手时只需要执行命令（代管的备份移动是本工具自己的保险）
    manual_exec_only: bool,
}

fn element_of(status: &Value, id: &str) -> Result<Value> {
    status
        .get("elements")
        .and_then(Value::as_array)
        .and_then(|els| {
            els.iter()
                .find(|e| e.get("program").and_then(Value::as_str) == Some(id))
        })
        .cloned()
        .ok_or_else(|| {
            anyhow!("找不到程序 {id}（可能已经没有技能或 MCP 需要它，或需要先重新扫描）")
        })
}

fn first_install<'a>(element: &'a Value, owner: &str) -> Option<&'a Value> {
    element
        .get("installs")
        .and_then(Value::as_array)?
        .iter()
        .find(|i| i.get("owner").and_then(Value::as_str) == Some(owner))
}

fn all_bins(element: &Value) -> Vec<String> {
    element
        .get("all_bins")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// 托管程序要清理的命令：状态里列出的，加上托管目录当前版本实际提供的。
fn managed_cleanup_bins(ctx: &Context, id: &str, element: Option<&Value>) -> Vec<String> {
    let mut bins = element.map(all_bins).unwrap_or_default();
    if let (Some(dir), Some((scheme, _))) =
        (super::status::managed_dir(ctx, id), id.split_once(':'))
    {
        for bin in super::status::managed_bins(&dir, scheme) {
            if !bins.contains(&bin) {
                bins.push(bin);
            }
        }
    }
    bins
}

/// npm 全局命令所在目录：Unix 为 `<prefix>/bin`，Windows 直接在 `<prefix>` 下（`%APPDATA%\npm\x.cmd`）。
fn npm_bin_dir(prefix: &Path) -> PathBuf {
    if cfg!(windows) {
        prefix.to_path_buf()
    } else {
        prefix.join("bin")
    }
}

/// Derive a global prefix only from the expected package suffix. Unix adds
/// `lib` before `node_modules`; Windows puts `node_modules` directly in it.
fn npm_global_prefix(package_dir: &Path, package: &str) -> Result<PathBuf> {
    let mut parent = package_dir;
    let parts: Vec<_> = package.split('/').collect();
    if !matches!(parts.as_slice(), [name] if !name.starts_with('@'))
        && !matches!(parts.as_slice(), [scope, _] if scope.starts_with('@'))
    {
        bail!("npm 包名结构认不出来");
    }
    for component in parts
        .iter()
        .rev()
        .copied()
        .chain(std::iter::once("node_modules"))
        .chain((!cfg!(windows)).then_some("lib"))
    {
        if component.is_empty() || parent.file_name() != Some(std::ffi::OsStr::new(component)) {
            bail!("npm 全局目录结构认不出来");
        }
        parent = parent
            .parent()
            .ok_or_else(|| anyhow!("npm 全局目录结构认不出来"))?;
    }
    if !parent.is_absolute() {
        bail!("npm 全局前缀不是绝对路径");
    }
    Ok(parent.to_path_buf())
}

/// 包目录下指向它的命令链接（npm 全局的 `<prefix>/bin/<命令>`、uv tool 的 `~/.local/bin/<命令>`）。
fn bin_links_into(bin_dir: &Path, package_dir: &Path, bins: &[String]) -> Vec<PathBuf> {
    let real_pkg = package_dir
        .canon()
        .unwrap_or_else(|_| package_dir.to_path_buf());
    bins.iter()
        .flat_map(|b| {
            let primary = bin_dir.join(crate::platform::command_link_name(b));
            if cfg!(windows) {
                vec![primary, bin_dir.join(format!("{b}.ps1")), bin_dir.join(b)]
            } else {
                vec![primary]
            }
        })
        .filter(|p| p.symlink_metadata().is_ok())
        .filter(|p| {
            // Windows 上是 npm 的 cmd-shim：顺着脚本里记的位置判断
            let followed = crate::platform::cmd_shim_target(p).unwrap_or_else(|| p.clone());
            let direct = followed
                .canon()
                .map(|r| r.starts_with(&real_pkg))
                .unwrap_or(false);
            if direct || !cfg!(windows) {
                return direct;
            }
            // npm also writes PowerShell and POSIX launchers beside its .cmd.
            // Only move scripts whose explicit $basedir target is in this package.
            if std::fs::metadata(p)
                .map(|m| m.len() > 64 * 1024)
                .unwrap_or(true)
            {
                return false;
            }
            let Ok(text) = std::fs::read_to_string(p) else {
                return false;
            };
            let re = regex::Regex::new(r#""\$basedir[\\/]([^"\r\n]+)""#)
                .expect("fixed npm shim pattern");
            let matches = re.captures_iter(&text).any(|capture| {
                let mut target = bin_dir.to_path_buf();
                for part in capture[1].split(['/', '\\']) {
                    target.push(part);
                }
                target
                    .canon()
                    .is_ok_and(|target| target.starts_with(&real_pkg))
            });
            matches
        })
        .collect()
}

fn backup_root(ctx: &Context) -> PathBuf {
    programs_root(ctx).join("backups")
}

fn new_backup_dir(ctx: &Context, id: &str) -> PathBuf {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%3f").to_string();
    let slug: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    backup_root(ctx).join(format!("{stamp}-{slug}"))
}

/// 代管改动前的备份：包目录与它的命令链接移进备份目录，并记下原位置。
fn backup_steps(
    backup: &Path,
    package_dir: &Path,
    links: &[PathBuf],
    id: &str,
    version: Option<&str>,
    reason: &str,
) -> Vec<Step> {
    let mut steps = vec![Step::Move {
        from: package_dir.to_path_buf(),
        to: backup.join("package"),
    }];
    let mut moved_links = Vec::new();
    for (i, link) in links.iter().enumerate() {
        let to = backup.join("bin").join(format!(
            "{i}-{}",
            link.file_name().unwrap_or_default().to_string_lossy()
        ));
        moved_links.push(json!({"from": link, "to": to}));
        steps.push(Step::Move {
            from: link.clone(),
            to,
        });
    }
    steps.push(Step::WriteBackupMeta {
        dir: backup.to_path_buf(),
        meta: json!({"program": id, "version": version, "reason": reason, "package_dir": package_dir, "links": moved_links, "created_at": chrono::Utc::now().to_rfc3339()}),
    });
    steps
}

/// 代管：用原包管理器把程序换到某个版本；改动前整包备份。
fn plan_delegated(
    ctx: &Context,
    status: &Value,
    element: &Value,
    args: &Args,
    user_path: &super::locate::UserPath,
) -> Result<Plan> {
    let id = &args.program;
    let (scheme, name) = id.split_once(':').unwrap_or_default();
    let owner = element
        .get("owner")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let install = first_install(element, owner).ok_or_else(|| anyhow!("{id} 不在 PATH 上"))?;
    let package_dir = install
        .get("package_dir")
        .and_then(Value::as_str)
        .map(|p| expand(p, &ctx.home_dir))
        .ok_or_else(|| anyhow!("{id} 的包目录认不出来"))?;
    let current = element
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string);
    let bins = all_bins(element);
    let _ = status;
    let backup = new_backup_dir(ctx, id);
    match owner {
        "npm-global" => {
            let npm = which("npm", user_path).ok_or_else(|| anyhow!("PATH 上没有 npm"))?;
            let target = match args.version.as_deref() {
                None | Some("latest") => npm_latest(&npm, name)?,
                Some(v) => v.to_string(),
            };
            let prefix = npm_global_prefix(&package_dir, name)?;
            let links = bin_links_into(&npm_bin_dir(&prefix), &package_dir, &bins);
            let allow = args.allow_scripts || scripts_allowed(status, id);
            let mut steps = backup_steps(
                &backup,
                &package_dir,
                &links,
                id,
                current.as_deref(),
                "upgrade",
            );
            let mut npm_args = vec![
                "install".to_string(),
                "-g".into(),
                "--prefix".into(),
                prefix.to_string_lossy().to_string(),
                "--no-fund".into(),
                "--no-audit".into(),
            ];
            if !allow {
                npm_args.push("--ignore-scripts".into());
            }
            npm_args.push(format!("{name}@{target}"));
            steps.push(Step::Exec {
                program: npm,
                args: npm_args,
                env: vec![],
                cwd: None,
            });
            Ok(Plan {
                program: id.clone(),
                action: args.action.clone(),
                target_version: Some(target.clone()),
                verify_bins: links
                    .iter()
                    .map(|l| {
                        (
                            l.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string(),
                            l.clone(),
                        )
                    })
                    .collect(),
                verify_version: Some((package_dir.join("package.json"), target)),
                steps,
                notes: vec![format!(
                    "改动前把 {} 整个移到备份，失败或回退时原样移回",
                    tilde(&package_dir, &ctx.home_dir)
                )],
                scripts_ignored: !allow,
                backup_dir: Some(backup),
                manual_exec_only: true,
            })
        }
        "uv-tool" => {
            if scheme != "pypi" {
                bail!("{id} 不是 PyPI 包，不能用 uv tool 换版本");
            }
            let uv = which("uv", user_path).ok_or_else(|| anyhow!("PATH 上没有 uv"))?;
            let target = match args.version.as_deref() {
                None | Some("latest") => pypi_latest(name, user_path)?,
                Some(v) => v.to_string(),
            };
            let tool_parent = package_dir
                .parent()
                .ok_or_else(|| anyhow!("uv tool 目录结构认不出来"))?
                .to_path_buf();
            let first_path = install
                .get("path")
                .and_then(Value::as_str)
                .map(|p| expand(p, &ctx.home_dir))
                .unwrap_or_default();
            let bin_dir = first_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| ctx.home_dir.join(".local/bin"));
            let links = bin_links_into(&bin_dir, &package_dir, &bins);
            let mut steps = backup_steps(
                &backup,
                &package_dir,
                &links,
                id,
                current.as_deref(),
                "upgrade",
            );
            steps.push(Step::Exec {
                program: uv,
                args: vec![
                    "tool".into(),
                    "install".into(),
                    "--force".into(),
                    format!("{name}=={target}"),
                ],
                env: vec![
                    (
                        "UV_TOOL_DIR".into(),
                        tool_parent.to_string_lossy().to_string(),
                    ),
                    (
                        "UV_TOOL_BIN_DIR".into(),
                        bin_dir.to_string_lossy().to_string(),
                    ),
                ],
                cwd: None,
            });
            Ok(Plan {
                program: id.clone(),
                action: args.action.clone(),
                target_version: Some(target),
                verify_bins: links
                    .iter()
                    .map(|l| {
                        (
                            l.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string(),
                            l.clone(),
                        )
                    })
                    .collect(),
                verify_version: None,
                steps,
                notes: vec![format!(
                    "改动前把 {} 整个移到备份，失败或回退时原样移回",
                    tilde(&package_dir, &ctx.home_dir)
                )],
                scripts_ignored: false,
                backup_dir: Some(backup),
                manual_exec_only: true,
            })
        }
        other => bail!("{id} 由 {other} 管理，本工具只能给出命令，不代为改动"),
    }
}

fn scripts_allowed(status: &Value, id: &str) -> bool {
    status
        .get("allow_scripts")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(id)))
}

/// 读一个备份的说明。
fn read_backup(ctx: &Context, backup: &str) -> Result<(PathBuf, Value)> {
    let dir = backup_root(ctx).join(backup);
    let meta: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join(BACKUP_META))
            .with_context(|| format!("找不到备份 {backup}"))?,
    )?;
    Ok((dir, meta))
}

/// 回退：把当前的包目录与命令链接挪进一份新备份，再把旧备份原样移回。
fn plan_restore(ctx: &Context, args: &Args) -> Result<Plan> {
    let backup_id = args
        .backup
        .as_deref()
        .ok_or_else(|| anyhow!("回退需要 --backup <备份编号>"))?;
    let (dir, meta) = read_backup(ctx, backup_id)?;
    if meta.get("program").and_then(Value::as_str) != Some(args.program.as_str()) {
        bail!("备份 {backup_id} 不是 {} 的", args.program);
    }
    let package_dir = PathBuf::from(
        meta.get("package_dir")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("备份说明不完整"))?,
    );
    let links: Vec<(PathBuf, PathBuf)> = meta
        .get("links")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|l| {
                    Some((
                        PathBuf::from(l.get("from")?.as_str()?),
                        PathBuf::from(l.get("to")?.as_str()?),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut steps = Vec::new();
    let aside = new_backup_dir(ctx, &args.program);
    if package_dir.exists() {
        let current_links: Vec<PathBuf> = links
            .iter()
            .map(|(from, _)| from.clone())
            .filter(|p| p.symlink_metadata().is_ok())
            .collect();
        let current_version = super::locate::read_version_at(&package_dir);
        steps.extend(backup_steps(
            &aside,
            &package_dir,
            &current_links,
            &args.program,
            current_version.as_deref(),
            "before-restore",
        ));
    }
    steps.push(Step::Move {
        from: dir.join("package"),
        to: package_dir.clone(),
    });
    for (from, to) in &links {
        steps.push(Step::Move {
            from: to.clone(),
            to: from.clone(),
        });
    }
    steps.push(Step::RemoveDir { path: dir.clone() });
    Ok(Plan {
        program: args.program.clone(),
        action: Action::Restore,
        target_version: meta
            .get("version")
            .and_then(Value::as_str)
            .map(str::to_string),
        verify_bins: links
            .iter()
            .map(|(from, _)| {
                (
                    from.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    from.clone(),
                )
            })
            .collect(),
        verify_version: None,
        steps,
        notes: vec!["零网络：只在本机移动目录".into()],
        scripts_ignored: false,
        backup_dir: Some(aside),
        manual_exec_only: false,
    })
}

/// 托管：装一个版本到本工具目录（不切换、不放入口链接，这些由调用方的计划决定）。
fn managed_install_steps(
    ctx: &Context,
    id: &str,
    version: &str,
    allow_scripts: bool,
    user_path: &super::locate::UserPath,
) -> Result<(Vec<Step>, PathBuf)> {
    let (scheme, name) = id.split_once(':').unwrap_or_default();
    let pdir = program_dir(ctx, id)?;
    let vdir = pdir.join(version);
    if vdir.join(META_FILE).is_file() {
        return Ok((Vec::new(), vdir));
    }
    let mut steps = Vec::new();
    if vdir.exists() {
        // 上次没装完：先清掉
        steps.push(Step::RemoveDir { path: vdir.clone() });
    }
    match scheme {
        "npm" => {
            let npm = which("npm", user_path).ok_or_else(|| anyhow!("PATH 上没有 npm"))?;
            let tmp = pdir.join(format!(".installing-{version}"));
            let mut npm_args = vec![
                "install".to_string(),
                "--prefix".into(),
                tmp.to_string_lossy().to_string(),
                "--cache".into(),
                programs_root(ctx)
                    .join("npm-cache")
                    .to_string_lossy()
                    .to_string(),
                "--no-save".into(),
                "--no-fund".into(),
                "--no-audit".into(),
                "--no-package-lock".into(),
            ];
            if !allow_scripts {
                npm_args.push("--ignore-scripts".into());
            }
            npm_args.push(format!("{name}@{version}"));
            if tmp.exists() {
                steps.push(Step::RemoveDir { path: tmp.clone() });
            }
            steps.push(Step::Exec {
                program: npm,
                args: npm_args,
                env: vec![],
                cwd: None,
            });
            steps.push(Step::Move {
                from: tmp,
                to: vdir.clone(),
            });
        }
        "pypi" | "git" => {
            let uv = which("uv", user_path).ok_or_else(|| anyhow!("PATH 上没有 uv"))?;
            // venv 里写死了绝对路径，不能装临时目录再改名：直接装到最终目录，元数据最后写入判完成
            let spec = if scheme == "pypi" {
                format!("{name}=={version}")
            } else {
                format!("git+https://{name}@{version}")
            };
            steps.push(Step::Exec {
                program: uv,
                args: vec!["tool".into(), "install".into(), "--force".into(), spec],
                env: vec![
                    (
                        "UV_TOOL_DIR".into(),
                        vdir.join("tools").to_string_lossy().to_string(),
                    ),
                    (
                        "UV_TOOL_BIN_DIR".into(),
                        vdir.join("bin").to_string_lossy().to_string(),
                    ),
                ],
                cwd: None,
            });
        }
        _ => bail!("{id} 不能由本工具安装"),
    }
    steps.push(Step::WriteMeta {
        dir: vdir.clone(),
        meta: json!({"program": id, "version": version, "scripts": allow_scripts, "installed_at": chrono::Utc::now().to_rfc3339()}),
    });
    Ok((steps, vdir))
}

/// 托管版本目录里命令的真实位置。
fn managed_bin_target(vdir_or_current: &Path, scheme: &str, bin: &str) -> PathBuf {
    // Windows：npm 在 .bin 下生成 `<命令>.cmd`，uv 在 bin 下放 `<命令>.exe`
    match (scheme, cfg!(windows)) {
        ("npm", false) => vdir_or_current.join("node_modules/.bin").join(bin),
        ("npm", true) => vdir_or_current
            .join("node_modules")
            .join(".bin")
            .join(format!("{bin}.cmd")),
        (_, false) => vdir_or_current.join("bin").join(bin),
        (_, true) => vdir_or_current.join("bin").join(format!("{bin}.exe")),
    }
}

/// 入口指向哪里：符号链接，或 Windows 的转发脚本。
fn link_target(path: &Path) -> Option<PathBuf> {
    crate::platform::command_link_target(path)
}

fn entry_links(
    ctx: &Context,
    id: &str,
    bins: &[String],
    entry_dir: &Path,
) -> Result<(Vec<Step>, Vec<(String, PathBuf)>)> {
    let (scheme, _) = id.split_once(':').unwrap_or_default();
    let pdir = program_dir(ctx, id)?;
    let bin_root = programs_root(ctx).join("bin");
    let mut steps = Vec::new();
    let mut verify = Vec::new();
    for bin in bins {
        let name = crate::platform::command_link_name(bin);
        let shim = bin_root.join(&name);
        steps.push(Step::Link {
            path: shim.clone(),
            target: managed_bin_target(&pdir.join("current"), scheme, bin),
            register: false,
        });
        let entry = entry_dir.join(&name);
        steps.push(Step::Link {
            path: entry.clone(),
            target: shim,
            register: true,
        });
        verify.push((bin.clone(), entry));
    }
    Ok((steps, verify))
}

fn resolve_entry_dir(ctx: &Context, status: &Value, args: &Args) -> Result<(PathBuf, bool)> {
    let saved = status.get("entry_dir").and_then(Value::as_str);
    let chosen = args.entry_dir.as_deref().or(saved).ok_or_else(|| anyhow!("还没有选入口目录：在 PATH 上选一个目录（例如 ~/.local/bin），本工具只在那里放登记过的链接"))?;
    let candidates: Vec<&str> = status
        .pointer("/user_path/entry_candidates")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if !candidates.contains(&chosen) {
        bail!("{chosen} 不在三种 shell 启动方式的 PATH 上都能看到，或不可写，不能做入口目录");
    }
    Ok((expand(chosen, &ctx.home_dir), saved != Some(chosen)))
}

fn plan_managed(
    ctx: &Context,
    status: &Value,
    element: Option<&Value>,
    args: &Args,
    user_path: &super::locate::UserPath,
) -> Result<Plan> {
    let id = &args.program;
    let (scheme, name) = id
        .split_once(':')
        .ok_or_else(|| anyhow!("程序标识格式不对"))?;
    let pdir = program_dir(ctx, id)?;
    let installed = super::status::managed_versions(&pdir).0;
    let bins: Vec<String> = element
        .map(all_bins)
        .filter(|b| !b.is_empty())
        .or_else(|| {
            element
                .and_then(|e| e.get("bins"))
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
        })
        .unwrap_or_default();
    let allow = args.allow_scripts || scripts_allowed(status, id);
    let latest = |args: &Args| -> Result<String> {
        match args.version.as_deref() {
            Some(v) if v != "latest" => Ok(v.to_string()),
            _ => match scheme {
                "npm" => npm_latest(
                    &which("npm", user_path).ok_or_else(|| anyhow!("PATH 上没有 npm"))?,
                    name,
                ),
                "pypi" => pypi_latest(name, user_path),
                _ => bail!("git 源要给出具体提交（--version <提交>）"),
            },
        }
    };
    let mut steps = Vec::new();
    let mut verify_bins = Vec::new();
    let mut notes = Vec::new();
    let mut target = None;
    match args.action {
        Action::Install | Action::Upgrade => {
            if bins.is_empty() {
                bail!("不知道 {id} 提供哪些命令，先补充声明");
            }
            let version = latest(args)?;
            let (install, _) = managed_install_steps(ctx, id, &version, allow, user_path)?;
            steps.extend(install);
            steps.push(Step::SetCurrent {
                program_dir: pdir.clone(),
                version: version.clone(),
            });
            let (entry_dir, save) = resolve_entry_dir(ctx, status, args)?;
            if save {
                steps.push(Step::SaveEntryDir {
                    dir: tilde(&entry_dir, &ctx.home_dir),
                });
            }
            let (links, verify) = entry_links(ctx, id, &bins, &entry_dir)?;
            steps.extend(links);
            verify_bins = verify;
            target = Some(version);
            if !installed.is_empty() {
                notes.push(format!(
                    "已有版本 {} 保留，可以随时切回",
                    installed.join("、")
                ));
            }
        }
        Action::Switch => {
            let version = args
                .version
                .clone()
                .ok_or_else(|| anyhow!("换版本需要 --version"))?;
            if !installed.contains(&version) {
                bail!("{id} 没有装过 {version}");
            }
            steps.push(Step::SetCurrent {
                program_dir: pdir.clone(),
                version: version.clone(),
            });
            target = Some(version);
        }
        Action::Remove => {
            let bins = managed_cleanup_bins(ctx, id, element);
            let links = registered_links(status);
            for (path, points_to) in links {
                if bins.iter().any(|b| {
                    expand(&points_to, &ctx.home_dir)
                        == programs_root(ctx)
                            .join("bin")
                            .join(crate::platform::command_link_name(b))
                }) {
                    steps.push(Step::Unlink {
                        path: expand(&path, &ctx.home_dir),
                    });
                }
            }
            for bin in &bins {
                steps.push(Step::Unlink {
                    path: programs_root(ctx)
                        .join("bin")
                        .join(crate::platform::command_link_name(bin)),
                });
            }
            if let Some(version) = args.version.clone() {
                // 只删一个版本：不能删当前版本
                let current = super::status::managed_current(&pdir)
                    .with_context(|| "无法确认当前托管版本，未删除任何版本")?
                    .ok_or_else(|| anyhow!("无法确认当前托管版本，未删除任何版本"))?;
                let same_target = pdir
                    .join(&version)
                    .canon()
                    .ok()
                    .zip(pdir.join(&current).canon().ok())
                    .is_some_and(|(requested, active)| requested == active);
                if current == version
                    || same_target
                    || cfg!(windows) && current.eq_ignore_ascii_case(&version)
                {
                    bail!("{version} 是当前版本，不能单独删除");
                }
                steps.clear();
                steps.push(Step::RemoveDir {
                    path: pdir.join(&version),
                });
            } else {
                steps.push(Step::RemoveDir { path: pdir.clone() });
            }
        }
        Action::Link | Action::Unlink => {
            let (entry_dir, save) = resolve_entry_dir(ctx, status, args)?;
            if args.action == Action::Link {
                if save {
                    steps.push(Step::SaveEntryDir {
                        dir: tilde(&entry_dir, &ctx.home_dir),
                    });
                }
                let (links, verify) = entry_links(ctx, id, &bins, &entry_dir)?;
                steps.extend(links);
                verify_bins = verify;
            } else {
                for bin in &bins {
                    steps.push(Step::Unlink {
                        path: entry_dir.join(bin),
                    });
                }
            }
        }
        _ => bail!("这个动作不适用于托管程序"),
    }
    if !allow && matches!(args.action, Action::Install | Action::Upgrade) && scheme == "npm" {
        notes.push(
            "安装脚本不执行（与 pnpm 10、npm 12 等的默认一致）；装好后若有包的脚本被跳过会列出来"
                .into(),
        );
    }
    Ok(Plan {
        program: id.clone(),
        action: args.action.clone(),
        target_version: target,
        steps,
        verify_bins,
        verify_version: None,
        notes,
        scripts_ignored: !allow && scheme == "npm",
        backup_dir: None,
        manual_exec_only: false,
    })
}

fn registered_links(status: &Value) -> Vec<(String, String)> {
    status
        .get("links")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// 纳入：代管 → 托管（同版本）。顺序保证 PATH 上任何时刻至少一份可用：装托管版本 → 放入口链接 →
/// 把外部包整包移到备份。撤回（unadopt）先把备份移回，再删托管。
fn plan_adopt(
    ctx: &Context,
    status: &Value,
    element: &Value,
    args: &Args,
    user_path: &super::locate::UserPath,
) -> Result<Plan> {
    let id = &args.program;
    let (scheme, name) = id.split_once(':').unwrap_or_default();
    let owner = element
        .get("owner")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if owner != "npm-global" || scheme != "npm" {
        bail!("目前只支持把 npm 全局安装的程序纳入托管（{id} 由 {owner} 管理）");
    }
    let install = first_install(element, owner).ok_or_else(|| anyhow!("{id} 不在 PATH 上"))?;
    let package_dir = install
        .get("package_dir")
        .and_then(Value::as_str)
        .map(|p| expand(p, &ctx.home_dir))
        .ok_or_else(|| anyhow!("包目录认不出来"))?;
    if package_dir
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        bail!("{name} 是用 npm link 装的，重装同一版本不等价，只能继续代管");
    }
    let version = element
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("{id} 的版本读不出来，不能纳入"))?
        .to_string();
    let bins = all_bins(element);
    let allow = args.allow_scripts || scripts_allowed(status, id);
    let pdir = program_dir(ctx, id)?;
    let mut steps = Vec::new();
    let (install_steps, _) = managed_install_steps(ctx, id, &version, allow, user_path)?;
    steps.extend(install_steps);
    steps.push(Step::SetCurrent {
        program_dir: pdir,
        version: version.clone(),
    });
    let (entry_dir, save) = resolve_entry_dir(ctx, status, args)?;
    if save {
        steps.push(Step::SaveEntryDir {
            dir: tilde(&entry_dir, &ctx.home_dir),
        });
    }
    let (links, verify) = entry_links(ctx, id, &bins, &entry_dir)?;
    steps.extend(links);
    let prefix = npm_global_prefix(&package_dir, name)?;
    let global_links = bin_links_into(&npm_bin_dir(&prefix), &package_dir, &bins);
    let backup = new_backup_dir(ctx, id);
    steps.extend(backup_steps(
        &backup,
        &package_dir,
        &global_links,
        id,
        Some(&version),
        "adopt",
    ));
    Ok(Plan {
        program: id.clone(),
        action: Action::Adopt,
        target_version: Some(version),
        steps,
        verify_bins: verify,
        verify_version: None,
        notes: vec![
            format!(
                "装好托管的同一版本、放好入口链接后，才把 {} 整个移到备份（不执行 npm uninstall）",
                tilde(&package_dir, &ctx.home_dir)
            ),
            "撤回在本机完成，不用联网".into(),
            "程序自己的配置与登录信息不动".into(),
        ],
        scripts_ignored: !allow,
        backup_dir: Some(backup),
        manual_exec_only: false,
    })
}

fn plan_unadopt(ctx: &Context, status: &Value, args: &Args) -> Result<Plan> {
    let mut restore = plan_restore(ctx, args)?;
    // 先移回外部安装，再删托管的入口链接、命令链接与版本目录
    let element = element_of(status, &args.program).ok();
    let bins = managed_cleanup_bins(ctx, &args.program, element.as_ref());
    for (path, points_to) in registered_links(status) {
        if bins.iter().any(|b| {
            expand(&points_to, &ctx.home_dir)
                == programs_root(ctx)
                    .join("bin")
                    .join(crate::platform::command_link_name(b))
        }) {
            restore.steps.push(Step::Unlink {
                path: expand(&path, &ctx.home_dir),
            });
        }
    }
    for bin in &bins {
        restore.steps.push(Step::Unlink {
            path: programs_root(ctx)
                .join("bin")
                .join(crate::platform::command_link_name(bin)),
        });
    }
    restore.steps.push(Step::RemoveDir {
        path: program_dir(ctx, &args.program)?,
    });
    restore.action = Action::Unadopt;
    // 之前移到一边的「当前包」其实就是托管前被移走、现在已移回的那份，不需要额外备份
    Ok(restore)
}

pub fn plan(ctx: &Context, req: &Request) -> Result<Plan> {
    let args = parse_args(req)?;
    let user_path = super::locate::user_path(ctx, false);
    let status = super::status::status(ctx, false)?;
    if args.action == Action::Restore {
        return plan_restore(ctx, &args);
    }
    if args.action == Action::Unadopt {
        return plan_unadopt(ctx, &status, &args);
    }
    let element = element_of(&status, &args.program).ok();
    let mode = element
        .as_ref()
        .and_then(|e| e.get("mode"))
        .and_then(Value::as_str)
        .unwrap_or("missing");
    let managed_dir_exists = program_dir(ctx, &args.program)
        .map(|d| d.exists())
        .unwrap_or(false);
    let mut plan = match (&args.action, mode) {
        (Action::Adopt, _) => plan_adopt(
            ctx,
            &status,
            element
                .as_ref()
                .ok_or_else(|| anyhow!("找不到程序 {}", args.program))?,
            &args,
            &user_path,
        )?,
        (Action::Upgrade | Action::Switch, "delegated") if !managed_dir_exists => plan_delegated(
            ctx,
            &status,
            element.as_ref().expect("有 mode 就有元素"),
            &args,
            &user_path,
        )?,
        (_, "observed") => bail!(
            "{} 由别的方式管理（Homebrew、系统或应用自带），本工具只观察",
            args.program
        ),
        _ => plan_managed(ctx, &status, element.as_ref(), &args, &user_path)?,
    };
    plan.notes.dedup();
    Ok(plan)
}

fn step_json(step: &Step, home: &Path) -> Value {
    let t = |p: &Path| tilde(p, home);
    match step {
        Step::Move { from, to } => json!({"kind": "move", "from": t(from), "to": t(to)}),
        Step::Exec {
            program, args, env, ..
        } => {
            json!({"kind": "exec", "command": std::iter::once(t(program)).chain(args.iter().cloned()).collect::<Vec<_>>().join(" "), "env": env.iter().map(|(k, v)| format!("{k}={}", tilde(Path::new(v), home))).collect::<Vec<_>>()})
        }
        Step::Link { path, target, .. } => {
            json!({"kind": "link", "path": t(path), "target": t(target)})
        }
        Step::Unlink { path } => json!({"kind": "unlink", "path": t(path)}),
        Step::RemoveDir { path } => json!({"kind": "remove", "path": t(path)}),
        Step::SetCurrent {
            program_dir,
            version,
        } => json!({"kind": "current", "path": t(program_dir), "version": version}),
        Step::WriteMeta { dir, .. } => json!({"kind": "meta", "path": t(dir)}),
        Step::WriteBackupMeta { dir, .. } => json!({"kind": "backup-meta", "path": t(dir)}),
        Step::SaveEntryDir { dir } => json!({"kind": "entry-dir", "dir": dir}),
    }
}

fn sh_quote(text: &str) -> String {
    if text.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(c, '/' | '.' | '-' | '_' | '@' | '=' | ':' | '~' | '+')
    }) {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

impl Plan {
    /// 同一件事自己在终端里做的命令（不想让本工具代办、或自动执行失败时用）。
    fn manual(&self, ctx: &Context) -> Vec<String> {
        let t = |p: &Path| sh_quote(&tilde(p, &ctx.home_dir)).replacen("'~/", "~/'", 1);
        let mut out = Vec::new();
        for step in &self.steps {
            if self.manual_exec_only && !matches!(step, Step::Exec { .. }) {
                continue;
            }
            match step {
                Step::Move { from, to } => {
                    if let Some(parent) = to.parent() {
                        out.push(format!("mkdir -p {}", t(parent)));
                    }
                    out.push(format!("mv {} {}", t(from), t(to)));
                }
                Step::Exec {
                    program,
                    args,
                    env,
                    cwd,
                } => {
                    let env: Vec<String> = env
                        .iter()
                        .map(|(k, v)| format!("{k}={}", t(Path::new(v))))
                        .collect();
                    let cd = cwd
                        .as_ref()
                        .map(|c| format!("cd {} && ", t(c)))
                        .unwrap_or_default();
                    let cmd = std::iter::once(
                        program
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default(),
                    )
                    .chain(
                        args.iter()
                            .map(|a| sh_quote(&tilde(Path::new(a), &ctx.home_dir))),
                    )
                    .collect::<Vec<_>>()
                    .join(" ");
                    out.push(format!(
                        "{cd}{}{cmd}",
                        if env.is_empty() {
                            String::new()
                        } else {
                            format!("{} ", env.join(" "))
                        }
                    ));
                }
                Step::Link { path, target, .. } => {
                    out.push(format!("ln -sfn {} {}", t(target), t(path)))
                }
                Step::Unlink { path } => out.push(format!("rm {}", t(path))),
                Step::RemoveDir { path } => out.push(format!("rm -rf {}", t(path))),
                Step::SetCurrent {
                    program_dir,
                    version,
                } => out.push(format!(
                    "ln -sfn {} {}",
                    sh_quote(version),
                    t(&program_dir.join("current"))
                )),
                Step::WriteMeta { .. }
                | Step::WriteBackupMeta { .. }
                | Step::SaveEntryDir { .. } => {}
            }
        }
        out
    }

    pub fn to_json(&self, ctx: &Context) -> Value {
        json!({
            "program": self.program,
            "action": format!("{:?}", self.action).to_lowercase(),
            "target_version": self.target_version,
            "steps": self.steps.iter().map(|s| step_json(s, &ctx.home_dir)).collect::<Vec<_>>(),
            "notes": self.notes,
            "scripts_ignored": self.scripts_ignored,
            "backup": self.backup_dir.as_ref().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().to_string()),
            "manual": self.manual(ctx),
        })
    }
}

// ---------------------------------------------------------------------------
// 执行
// ---------------------------------------------------------------------------

/// 入口链接只能放在「不存在」或「本工具登记过、仍指向本工具目录」的位置。
fn check_link_slot(ctx: &Context, path: &Path, registered: &[(String, String)]) -> Result<()> {
    if path.symlink_metadata().is_err() {
        return Ok(());
    }
    let key = tilde(path, &ctx.home_dir);
    let ours = link_target(path).is_some_and(|t| t.starts_with(programs_root(ctx)))
        && (path.starts_with(programs_root(ctx)) || registered.iter().any(|(p, _)| *p == key));
    if !ours {
        bail!("{key} 已经有别的文件，没有覆盖（换一个入口目录，或自己处理这个文件）");
    }
    Ok(())
}

fn symlink(target: &Path, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_file_name(format!(
        ".{}.am-tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    let _ = crate::platform::remove_link(&tmp);
    // 相对目标（`current` → 版本号）按链接所在目录解析，判断是目录还是文件
    let resolved = path.parent().unwrap_or(Path::new(".")).join(target);
    if resolved.is_dir() {
        crate::platform::link_dir(target, &tmp)?;
    } else {
        crate::platform::link_file(target, &tmp)?;
    }
    crate::platform::replace_link(&tmp, path)?;
    Ok(())
}

/// 装好的 npm 目录里，哪些包带安装脚本（被跳过的就是这些）。
fn packages_with_scripts(vdir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let nm = vdir.join("node_modules");
    let Ok(rd) = std::fs::read_dir(&nm) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = Vec::new();
    for e in rd.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('@') {
            if let Ok(inner) = std::fs::read_dir(e.path()) {
                dirs.extend(inner.filter_map(|x| x.ok()).map(|x| x.path()));
            }
        } else if !name.starts_with('.') {
            dirs.push(e.path());
        }
    }
    for dir in dirs {
        let Ok(text) = std::fs::read_to_string(dir.join("package.json")) else {
            continue;
        };
        let Ok(pkg) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let scripts = pkg.get("scripts");
        let has = ["preinstall", "install", "postinstall"]
            .iter()
            .any(|k| scripts.and_then(|s| s.get(*k)).is_some())
            || dir.join("binding.gyp").is_file();
        if has {
            out.push(
                pkg.get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            );
        }
    }
    out.sort();
    out
}

pub fn execute(ctx: &Context, req: &Request) -> Result<Value> {
    let plan = plan(ctx, req)?;
    let status = super::status::status(ctx, false)?;
    let registered = registered_links(&status);
    // 执行前先查所有入口链接位置，冲突就一步都不做
    for step in &plan.steps {
        if let Step::Link {
            path,
            register: true,
            ..
        } = step
        {
            check_link_slot(ctx, path, &registered)?;
        }
    }
    let mut done: Vec<Step> = Vec::new();
    let mut new_links: Vec<(String, String)> = Vec::new();
    let mut removed_links: Vec<String> = Vec::new();
    let result = (|| -> Result<()> {
        for step in &plan.steps {
            match step {
                Step::Move { from, to } => {
                    if let Some(parent) = to.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::rename(from, to)
                        .with_context(|| format!("移动 {} 失败", tilde(from, &ctx.home_dir)))?;
                }
                Step::Exec {
                    program,
                    args,
                    env,
                    cwd,
                } => {
                    let mut cmd = crate::process::command_with_env(
                        program,
                        env.iter()
                            .map(|(key, value)| (key.as_str(), value.as_str())),
                    );
                    cmd.args(args);
                    if let Some(cwd) = cwd {
                        cmd.current_dir(cwd);
                    }
                    let out = run(cmd, INSTALL_TIMEOUT)?;
                    if !out.ok {
                        bail!("{} 失败：\n{}", tilde(program, &ctx.home_dir), out.tail);
                    }
                }
                Step::Link {
                    path,
                    target,
                    register,
                } => {
                    // 指向命令文件的入口：Windows 上写转发脚本（npm 的 .cmd 不能链接到别处）
                    if cfg!(windows) && !path.ends_with("current") {
                        if let Some(parent) = path.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        crate::platform::write_cmd_shim(target, path)?;
                    } else {
                        symlink(target, path)?;
                    }
                    if *register {
                        new_links.push((tilde(path, &ctx.home_dir), tilde(target, &ctx.home_dir)));
                    }
                }
                Step::Unlink { path } => {
                    if path.symlink_metadata().is_ok() {
                        let points_here =
                            link_target(path).is_some_and(|t| t.starts_with(programs_root(ctx)));
                        if points_here {
                            crate::platform::remove_link(path)?;
                            removed_links.push(tilde(path, &ctx.home_dir));
                        }
                    }
                }
                Step::RemoveDir { path } => {
                    if !path.starts_with(programs_root(ctx)) {
                        bail!("只删除本工具目录下的内容");
                    }
                    if path.exists() {
                        std::fs::remove_dir_all(path)?;
                    }
                }
                Step::SetCurrent {
                    program_dir,
                    version,
                } => symlink(Path::new(version), &program_dir.join("current"))?,
                Step::WriteMeta { dir, meta } => {
                    std::fs::create_dir_all(dir)?;
                    std::fs::write(dir.join(META_FILE), serde_json::to_vec_pretty(meta)?)?;
                }
                Step::WriteBackupMeta { dir, meta } => {
                    std::fs::create_dir_all(dir)?;
                    std::fs::write(dir.join(BACKUP_META), serde_json::to_vec_pretty(meta)?)?;
                }
                Step::SaveEntryDir { dir } => {
                    let dir = dir.clone();
                    update_programs_policy(ctx, req, "entry_dir", |p| {
                        p.insert("entry_dir".into(), json!(dir));
                    })?;
                }
            }
            done.push(step.clone());
        }
        // 核对：该找到的命令都在，版本对得上
        for (bin, path) in &plan.verify_bins {
            if !path.exists() {
                bail!("装完后找不到命令 {bin}（{}）", tilde(path, &ctx.home_dir));
            }
        }
        if let Some((package_json, version)) = &plan.verify_version {
            let got = std::fs::read_to_string(package_json)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| v.get("version").and_then(Value::as_str).map(str::to_string));
            if got.as_deref() != Some(version.as_str()) {
                bail!(
                    "装完后版本是 {}，不是 {version}",
                    got.unwrap_or_else(|| "（读不到）".into())
                );
            }
        }
        Ok(())
    })();
    if let Err(err) = result {
        // 失败回滚：按相反顺序撤回已做的移动与链接；新装出来的包目录（在备份位置之外）删掉
        for step in done.iter().rev() {
            match step {
                Step::Move { from, to } => {
                    if from.exists() {
                        // 原位置上已经有了新装的：先删掉（只在本工具备份流程里发生，原内容在 to）
                        if from
                            .symlink_metadata()
                            .map(|m| m.file_type().is_symlink())
                            .unwrap_or(false)
                        {
                            let _ = crate::platform::remove_link(from);
                        } else if from.is_dir() {
                            let _ = std::fs::remove_dir_all(from);
                        } else {
                            let _ = std::fs::remove_file(from);
                        }
                    }
                    let _ = std::fs::rename(to, from);
                }
                Step::Link { path, .. } => {
                    let _ = crate::platform::remove_link(path);
                }
                _ => {}
            }
        }
        // 已经原样移回：这次的备份记录没有内容了，删掉
        if let Some(backup) = &plan.backup_dir {
            if backup.exists() && !backup.join("package").exists() {
                let _ = std::fs::remove_dir_all(backup);
            }
        }
        let _ = record(
            ctx,
            req,
            &plan.program,
            json!({"action": format!("{:?}", plan.action).to_lowercase(), "ok": false}),
        );
        return Err(err);
    }
    // 登记新放的入口链接、去掉删掉的；已经不指向本工具目录的旧登记一并去掉
    for (path, _) in &registered {
        let abs = expand(path, &ctx.home_dir);
        let ours = link_target(&abs).is_some_and(|t| t.starts_with(programs_root(ctx)));
        if !ours && !removed_links.contains(path) {
            removed_links.push(path.clone());
        }
    }
    if !new_links.is_empty() || !removed_links.is_empty() {
        update_programs_policy(ctx, req, "links", |p| {
            let links = p.entry("links").or_insert_with(|| json!({}));
            if !links.is_object() {
                *links = json!({});
            }
            let map = links.as_object_mut().expect("刚确保是对象");
            for path in &removed_links {
                map.remove(path);
            }
            for (path, target) in &new_links {
                map.insert(path.clone(), json!(target));
            }
        })?;
    }
    if plan.scripts_ignored {
        // 装好后列出被跳过脚本的包
    }
    prune_backups(ctx, &plan.program);
    let skipped = match plan.target_version.as_deref() {
        Some(v) if plan.scripts_ignored => program_dir(ctx, &plan.program)
            .map(|d| packages_with_scripts(&d.join(v)))
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    record(
        ctx,
        req,
        &plan.program,
        json!({"action": format!("{:?}", plan.action).to_lowercase(), "ok": true, "version": plan.target_version}),
    )?;
    let mut out = plan.to_json(ctx);
    out["scripts_skipped"] = json!(skipped);
    Ok(out)
}

/// 同一程序只留最近几份备份；纳入时的备份（撤回要用）不清。
fn prune_backups(ctx: &Context, id: &str) {
    let Ok(rd) = std::fs::read_dir(backup_root(ctx)) else {
        return;
    };
    let mut mine: Vec<(String, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let meta: Value =
                serde_json::from_str(&std::fs::read_to_string(e.path().join(BACKUP_META)).ok()?)
                    .ok()?;
            (meta.get("program").and_then(Value::as_str) == Some(id)
                && meta.get("reason").and_then(Value::as_str) != Some("adopt"))
            .then(|| (e.file_name().to_string_lossy().to_string(), e.path()))
        })
        .collect();
    mine.sort();
    while mine.len() > KEEP_BACKUPS {
        let (_, path) = mine.remove(0);
        let _ = std::fs::remove_dir_all(path);
    }
}

/// 某个程序的备份（新的在前）：回退与撤回纳入时选用。
pub fn backups(ctx: &Context) -> Vec<Value> {
    let Ok(rd) = std::fs::read_dir(backup_root(ctx)) else {
        return Vec::new();
    };
    let mut out: Vec<(String, Value)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let meta: Value = serde_json::from_str(&std::fs::read_to_string(e.path().join(BACKUP_META)).ok()?).ok()?;
            let id = e.file_name().to_string_lossy().to_string();
            Some((id.clone(), json!({"id": id, "program": meta.get("program"), "version": meta.get("version"), "reason": meta.get("reason"), "created_at": meta.get("created_at")})))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.into_iter().map(|(_, v)| v).collect()
}

/// 保存补充声明、安装方式、允许安装脚本（只写库）。
pub fn declare(ctx: &Context, req: &Request) -> Result<Value> {
    let known = [
        "--skill",
        "--bins",
        "--bin",
        "--program",
        "--allow-scripts",
        "--server",
        "--follow-latest",
    ];
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--json" {
            continue;
        }
        if !known.contains(&arg.as_str()) || iter.next().is_none() {
            bail!("programs:declare 参数不对：{arg}");
        }
    }
    // MCP 服务选择跟最新版本（取消固定）或回到固定：只记在库里，条目的改写走 MCP 编辑
    if let Some(server) = req.value("--server") {
        let follow = match req.value("--follow-latest") {
            Some("true") => true,
            Some("false") => false,
            _ => bail!("--server 需要 --follow-latest true|false"),
        };
        if server.is_empty()
            || !server
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            bail!("服务名不合法");
        }
        let server = server.to_string();
        update_programs_policy(ctx, req, "follow_latest", |p| {
            let list = p.entry("follow_latest").or_insert_with(|| json!([]));
            if !list.is_array() {
                *list = json!([]);
            }
            let arr = list.as_array_mut().expect("数组");
            arr.retain(|v| v.as_str() != Some(server.as_str()));
            if follow {
                arr.push(json!(server));
            }
        })?;
        return Ok(json!({"server": req.value("--server"), "follow_latest": follow}));
    }
    if let Some(skill) = req.value("--skill") {
        let bins: Vec<String> = serde_json::from_str(
            req.value("--bins")
                .ok_or_else(|| anyhow!("需要 --bins <JSON 数组>"))?,
        )?;
        if bins.iter().any(|b| !super::declare::valid_bin(b)) {
            bail!("命令名不合法");
        }
        let skill = skill.to_string();
        update_programs_policy(ctx, req, "declarations", |p| {
            let decl = p.entry("declarations").or_insert_with(|| json!({}));
            if !decl.is_object() {
                *decl = json!({});
            }
            let map = decl.as_object_mut().expect("对象");
            if bins.is_empty() {
                map.remove(&skill);
            } else {
                map.insert(skill.clone(), json!({"bins": bins}));
            }
        })?;
        return Ok(json!({"declared": req.value("--skill")}));
    }
    if let Some(bin) = req.value("--bin") {
        let program = req
            .value("--program")
            .ok_or_else(|| anyhow!("需要 --program <程序标识>"))?
            .to_string();
        if !super::declare::valid_bin(bin) || !program.contains(':') {
            bail!("命令名或程序标识不合法");
        }
        let bin = bin.to_string();
        update_programs_policy(ctx, req, "install_methods", |p| {
            let m = p.entry("install_methods").or_insert_with(|| json!({}));
            if !m.is_object() {
                *m = json!({});
            }
            m.as_object_mut()
                .expect("对象")
                .insert(bin.clone(), json!(program));
        })?;
        return Ok(json!({"bin": req.value("--bin")}));
    }
    if let Some(program) = req.value("--program") {
        let allow = req.value("--allow-scripts") == Some("true");
        let program = program.to_string();
        update_programs_policy(ctx, req, "allow_scripts", |p| {
            let list = p.entry("allow_scripts").or_insert_with(|| json!([]));
            if !list.is_array() {
                *list = json!([]);
            }
            let arr = list.as_array_mut().expect("数组");
            arr.retain(|v| v.as_str() != Some(program.as_str()));
            if allow {
                arr.push(json!(program));
            }
        })?;
        return Ok(json!({"program": req.value("--program"), "allow_scripts": allow}));
    }
    bail!("programs:declare 需要 --skill、--bin 或 --program")
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn npm_global_prefix_handles_ordinary_and_scoped_native_layouts() {
        let prefix = std::env::temp_dir().join("npm-prefix");
        let modules = if cfg!(windows) {
            prefix.join("node_modules")
        } else {
            prefix.join("lib/node_modules")
        };
        for package in ["tool-a", "@scope/tool-a"] {
            assert_eq!(
                npm_global_prefix(&modules.join(package), package).unwrap(),
                prefix
            );
            assert!(npm_global_prefix(&modules.join(package), "other").is_err());
        }
        assert!(npm_global_prefix(&prefix.join("packages/tool-a"), "tool-a").is_err());
    }

    #[test]
    fn version_cannot_address_parent_or_current_pointer() {
        for version in [
            ".", "..", "current", "CURRENT", "current.", "1.2.0.", "1.2.0 ", "NUL", "con.txt",
            "COM1", "lpt9.0", "../1.0.0", "1.0.0/..",
        ] {
            assert!(!valid_version(version), "accepted {version}");
        }
        assert!(valid_version("1.2.0-rc.1"));
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use crate::process::capture_test_support as fixture;
    use std::time::Instant;

    fn inherited_pipe_deadline(stream: &str, timeout: Duration) {
        let root = fixture::root(stream);
        let start = Instant::now();
        let result = run(fixture::command(&root, stream), timeout);
        let elapsed = start.elapsed();
        assert!(
            elapsed < timeout + Duration::from_secs(1),
            "Waited for an inherited {stream} pipe: {elapsed:?}"
        );
        assert!(result.err().unwrap().to_string().contains("超时"));
        fixture::assert_descendant_stopped(&root, start);
    }

    #[test]
    fn run_times_out_after_parent_exits_with_inherited_stdout() {
        inherited_pipe_deadline("stdout", Duration::from_secs(2));
    }

    #[test]
    fn run_times_out_after_parent_exits_with_inherited_stderr() {
        inherited_pipe_deadline("stderr", Duration::from_secs(2));
    }

    #[test]
    fn run_uses_one_deadline_for_parent_and_reader_completion() {
        inherited_pipe_deadline("delayed-stdout", Duration::from_secs(3));
    }

    #[test]
    fn run_preserves_exit_status_output_and_diagnostic_tail() {
        let root = fixture::root("output");
        let output = run(fixture::command(&root, "output"), Duration::from_secs(10)).unwrap();
        assert!(!output.ok);
        assert!(output.stdout.contains("retained standard output"));
        assert_eq!(
            output.tail,
            (2..10)
                .map(|i| format!("diagnostic-{i}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn run_drains_both_streams_past_the_capture_limit() {
        let root = fixture::root("limit");
        let output = run(
            fixture::command(&root, "large-output"),
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(output.ok);
        assert_eq!(output.stdout.len(), 4 * 1024 * 1024);
        assert!(output.stdout.ends_with("oooo"));
        assert!(output.tail.ends_with("eeee"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
