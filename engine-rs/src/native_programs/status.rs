//! `programs:status`：技能与 MCP 需要哪些程序、装在哪、归谁管、什么版本、有什么问题。
//!
//! 依赖从三处来（docs/PROGRAM-LIFECYCLE.md 第 6 节）：各账号在用技能的 SKILL.md 声明、各账号
//! MCP 条目的启动命令、用户确认的补充声明（库里 `programs.declarations` / `programs.install_methods`）。
//! 同一个包提供的命令合成一个程序。智能体自己（Claude Code、Codex）不在范围内。
//!
//! 只读零写、零网络，不执行被检测的程序（新版本探测结果只从探测缓存读）。

use crate::platform::Canon;
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

use super::declare::{self, InstallSpec};
use super::locate::{self, Located, UserPath};
use crate::native::Context;
use crate::store;

/// 一条依赖边：谁（技能或 MCP 服务）需要什么。
#[derive(Clone, Debug)]
struct Dependent {
    kind: &'static str,
    /// 技能名或 MCP 服务名。
    name: String,
    agents: Vec<String>,
    /// declared（技能自己写的）/ user（用户确认的补充声明）/ mcp（从启动命令看出的）
    source: &'static str,
    bins: Vec<String>,
    /// 任选其一即可（OpenClaw 的 anyBins）。
    any: bool,
}

/// MCP 服务经 npx / uvx 临时拉取运行：版本写在条目里。
#[derive(Clone, Debug)]
struct Runner {
    server: String,
    agent: String,
    runner: &'static str,
    /// 条目里写的版本（npx 的 `@版本`、uvx 的 `==版本`、git 的 `@引用`）；没写为 None。
    version: Option<String>,
    /// 版本是否固定：精确版本号或 40 位提交为真；`latest`、分支名、没写为假。
    pinned: bool,
}

#[derive(Default)]
struct Program {
    id: String,
    bins: Vec<String>,
    dependents: Vec<Dependent>,
    install: Vec<InstallSpec>,
    runners: Vec<Runner>,
    /// 命令 → 在 PATH 上的全部位置
    located: BTreeMap<String, Vec<Located>>,
}

fn policy_object(policy: &Value, key: &str) -> Map<String, Value> {
    policy
        .pointer(&format!("/programs/{key}"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// 精确版本：数字开头、只含版本字符；或 40 位提交号。
fn is_exact(version: &str) -> bool {
    let commit = version.len() == 40 && version.chars().all(|c| c.is_ascii_hexdigit());
    let semver = version.chars().next().is_some_and(|c| c.is_ascii_digit())
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    commit || semver
}

/// `@scope/name@1.2.3` → (`@scope/name`, Some(`1.2.3`))
fn split_npm_spec(spec: &str) -> (String, Option<String>) {
    let (name, version) = match spec.strip_prefix('@') {
        Some(rest) => match rest.find('@') {
            Some(pos) => (&spec[..pos + 1], Some(&rest[pos + 1..])),
            None => (spec, None),
        },
        None => match spec.split_once('@') {
            Some((name, version)) => (name, Some(version)),
            None => (spec, None),
        },
    };
    (
        name.to_string(),
        version.filter(|v| !v.is_empty()).map(str::to_string),
    )
}

/// MCP 条目的启动命令 → (程序标识, 运行方式)；普通命令名 → 需要的命令；绝对路径 → 按位置判归属。
enum McpNeed {
    Runner(String, Runner),
    Bin(String),
    Path(String),
}

fn mcp_need(server: &str, agent: &str, command: &str, args: &[String]) -> Option<McpNeed> {
    // 绝对路径取文件名（Windows 路径用 `\`），去掉 Windows 的可执行扩展名（`npx.cmd` → `npx`）
    let file = command.rsplit(['/', '\\']).next().unwrap_or(command);
    let base = [".cmd", ".exe", ".bat"]
        .iter()
        .find_map(|ext| {
            file.len()
                .checked_sub(ext.len())
                .filter(|&at| file[at..].eq_ignore_ascii_case(ext))
                .map(|at| &file[..at])
        })
        .unwrap_or(file);
    let positional = |skip_values: &[&str]| -> Vec<String> {
        let mut out = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if skip_values.contains(&arg.as_str()) {
                iter.next();
                continue;
            }
            if !arg.starts_with('-') {
                out.push(arg.clone());
            }
        }
        out
    };
    match base {
        "npx" => {
            let spec = args
                .iter()
                .position(|a| a == "--package" || a == "-p")
                .and_then(|i| args.get(i + 1))
                .cloned()
                .or_else(|| positional(&[]).into_iter().next())?;
            let (package, version) = split_npm_spec(&spec);
            let pinned = version.as_deref().is_some_and(is_exact);
            Some(McpNeed::Runner(
                format!("npm:{package}"),
                Runner {
                    server: server.to_string(),
                    agent: agent.to_string(),
                    runner: "npx",
                    version,
                    pinned,
                },
            ))
        }
        "uvx" => {
            let from = args
                .iter()
                .position(|a| a == "--from")
                .and_then(|i| args.get(i + 1))
                .cloned();
            let target = match from {
                Some(from) => from,
                None => positional(&["--python", "--with", "--index", "--index-url"])
                    .into_iter()
                    .next()?,
            };
            if let Some(git) = target.strip_prefix("git+") {
                let (repo, reference) = match git.rsplit_once('@') {
                    Some((repo, reference)) if !repo.ends_with(':') => {
                        (repo.to_string(), Some(reference.to_string()))
                    }
                    _ => (git.to_string(), None),
                };
                let pinned = reference
                    .as_deref()
                    .is_some_and(|r| r.len() == 40 && r.chars().all(|c| c.is_ascii_hexdigit()));
                let repo = repo
                    .trim_end_matches(".git")
                    .trim_start_matches("https://")
                    .to_string();
                return Some(McpNeed::Runner(
                    format!("git:{repo}"),
                    Runner {
                        server: server.to_string(),
                        agent: agent.to_string(),
                        runner: "uvx",
                        version: reference,
                        pinned,
                    },
                ));
            }
            let (package, version) = match target.split_once("==") {
                Some((package, version)) => (package.to_string(), Some(version.to_string())),
                None => (
                    target
                        .split(['@', '['])
                        .next()
                        .unwrap_or(&target)
                        .to_string(),
                    None,
                ),
            };
            let pinned = version.as_deref().is_some_and(is_exact);
            Some(McpNeed::Runner(
                format!("pypi:{package}"),
                Runner {
                    server: server.to_string(),
                    agent: agent.to_string(),
                    runner: "uvx",
                    version,
                    pinned,
                },
            ))
        }
        _ if command.starts_with('/') || Path::new(command).is_absolute() => {
            Some(McpNeed::Path(command.to_string()))
        }
        _ if declare::valid_bin(command) => Some(McpNeed::Bin(command.to_string())),
        _ => None,
    }
}

const META_FILE: &str = ".agent-manager-program.json";

/// 数据目录里托管着的程序标识（`programs/<类别>/<名>`，名里的 `~` 还原成 `/`）。
fn managed_ids(ctx: &Context) -> Vec<String> {
    let root = ctx.data_dir.join("programs");
    let mut out = Vec::new();
    for scheme in ["npm", "pypi", "git"] {
        let Ok(rd) = std::fs::read_dir(root.join(scheme)) else {
            continue;
        };
        for entry in rd.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().replace('~', "/");
            if !name.starts_with('.') && !managed_versions(&entry.path()).0.is_empty() {
                out.push(format!("{scheme}:{name}"));
            }
        }
    }
    out
}

/// Resolve both relative symlinks and absolute Windows junctions to a completed,
/// direct child version. An unreadable or external pointer is never a version ID.
pub(crate) fn managed_current(dir: &Path) -> Result<Option<String>> {
    let current = dir.join("current");
    match std::fs::symlink_metadata(&current) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    // A normal directory named `current` is not a managed version pointer.
    crate::platform::read_link(&current)?;
    let root = dir.canon()?;
    let target = current.canon()?;
    if target.parent() != Some(root.as_path()) || !target.join(META_FILE).is_file() {
        bail!("当前版本不在已完成的托管版本目录内");
    }
    let version = target
        .file_name()
        .and_then(|v| v.to_str())
        .filter(|v| *v != "current" && !v.starts_with('.'))
        .ok_or_else(|| anyhow!("当前版本目录无法识别"))?;
    Ok(Some(version.to_string()))
}

/// 托管的版本（装完整的）与当前版本。
pub(crate) fn managed_versions(dir: &Path) -> (Vec<String>, Option<String>) {
    let versions: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name() != "current"
                        && e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                        && e.path().join(META_FILE).is_file()
                })
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    let current = managed_current(dir).ok().flatten();
    (versions, current)
}

/// 托管程序当前版本提供的命令。
pub(crate) fn managed_bins(dir: &Path, scheme: &str) -> Vec<String> {
    let bin_dir = match scheme {
        "npm" => dir.join("current/node_modules/.bin"),
        _ => dir.join("current/bin"),
    };
    let mut out: Vec<String> = std::fs::read_dir(bin_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| !cfg!(windows) || crate::platform::is_executable(&e.path()))
                .map(|e| {
                    crate::platform::command_name(&e.file_name().to_string_lossy()).to_string()
                })
                .filter(|n| !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out.dedup();
    out
}

pub(crate) fn managed_dir(ctx: &Context, id: &str) -> Option<std::path::PathBuf> {
    let (scheme, name) = id.split_once(':')?;
    matches!(scheme, "npm" | "pypi" | "git").then(|| {
        ctx.data_dir
            .join("programs")
            .join(scheme)
            .join(name.replace('/', "~"))
    })
}

fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", crate::platform::slash(&rest)),
        Err(_) => path.to_string_lossy().to_string(),
    }
}

pub fn status(ctx: &Context, refresh_path: bool) -> Result<Value> {
    let policy = store::effective_policy(ctx)?;
    let scope = super::scope::Scope::new(&policy);
    let user_path = locate::user_path(ctx, refresh_path);
    let install_methods = policy_object(&policy, "install_methods");
    // 用户主动选择「跟最新版本」的 MCP 服务：不再报「没固定版本」
    let follow_latest: Vec<String> = policy
        .pointer("/programs/follow_latest")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let user_declarations = policy_object(&policy, "declarations");

    let mut programs: BTreeMap<String, Program> = BTreeMap::new();
    let mut located_cache: BTreeMap<String, Vec<Located>> = BTreeMap::new();
    let mut locate_bin = |bin: &str| -> Vec<Located> {
        located_cache
            .entry(bin.to_string())
            .or_insert_with(|| {
                locate::find_all(&user_path, bin)
                    .iter()
                    .map(|p| locate::classify(ctx, p))
                    .collect()
            })
            .clone()
    };
    let mut suggestions: Vec<Value> = Vec::new();

    // 一条「需要命令 bin」的依赖归到哪个程序：安装方式 > 在 PATH 上找到的包 > 用户补充的安装方式 > 只知道命令名
    let place = |programs: &mut BTreeMap<String, Program>,
                 bin: &str,
                 install: &[InstallSpec],
                 dep: &Dependent,
                 locate_bin: &mut dyn FnMut(&str) -> Vec<Located>|
     -> Option<String> {
        if scope.assistant_bin(bin) {
            return None;
        }
        let located = locate_bin(bin);
        if located.iter().any(|l| l.owner == "agent") {
            return None; // 智能体自己的程序不管
        }
        let spec = install
            .iter()
            .find(|s| s.bins.iter().any(|b| b == bin))
            .or_else(|| (install.len() == 1).then(|| &install[0]));
        let id = spec
            .map(InstallSpec::program_id)
            // PATH 上可能有几份同名的：认出包的那一份决定归属（同名多份另报「已偏离」）
            .or_else(|| located.iter().find_map(|l| l.program.clone()))
            .or_else(|| {
                install_methods
                    .get(bin)
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("bin:{bin}"));
        if scope.assistant_program(&id) {
            return None;
        }
        let program = programs.entry(id.clone()).or_insert_with(|| Program {
            id: id.clone(),
            ..Default::default()
        });
        if !program.bins.iter().any(|b| b == bin) {
            program.bins.push(bin.to_string());
        }
        if let Some(spec) = spec {
            if !program.install.contains(spec) {
                program.install.push(spec.clone());
            }
        }
        program.located.insert(bin.to_string(), located);
        if !program
            .dependents
            .iter()
            .any(|d| d.kind == dep.kind && d.name == dep.name)
        {
            program.dependents.push(dep.clone());
        }
        Some(id)
    };

    // 1. 各账号在用技能的声明与补充声明
    for skill in crate::native_skills::skills_in_use(ctx)? {
        let text = std::fs::read_to_string(skill.dir.join("SKILL.md")).unwrap_or_default();
        let declared = declare::parse_skill_md(&text);
        let extra: Vec<String> = user_declarations
            .get(&skill.name)
            .and_then(|d| d.get("bins"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter(|b| declare::valid_bin(b))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        for (bins, any, source) in [
            (&declared.bins, false, "declared"),
            (&declared.any_bins, true, "declared"),
            (&extra, false, "user"),
        ] {
            if bins.is_empty() {
                continue;
            }
            let dep = Dependent {
                kind: "skill",
                name: skill.name.clone(),
                agents: skill.agents.clone(),
                source,
                bins: bins.clone(),
                any,
            };
            for bin in bins {
                place(&mut programs, bin, &declared.install, &dep, &mut locate_bin);
            }
        }
        let known: Vec<&String> = declared
            .bins
            .iter()
            .chain(&declared.any_bins)
            .chain(&extra)
            .collect();
        for s in declare::suggestions(&text) {
            if scope.assistant_bin(&s.bin)
                || s.program
                    .as_deref()
                    .is_some_and(|id| scope.assistant_program(id))
                || known.iter().any(|b| **b == s.bin)
                || locate_bin(&s.bin).iter().any(|l| l.owner == "agent")
            {
                continue;
            }
            let found = !locate_bin(&s.bin).is_empty();
            suggestions.push(json!({"skill": skill.name, "bin": s.bin, "program": s.program, "line": s.line, "found": found}));
        }
    }

    // 2. 各账号的 MCP 条目
    let mcp = crate::native_mcp::inventory::load(ctx)?;
    for scan in &mcp.scans {
        for entry in &scan.entries {
            let Ok(view) = &entry.view else { continue };
            if view.command.is_empty() || scope.assistant_bin(&view.command) {
                continue;
            }
            let agent = scan.target.agent.clone();
            let Some(need) = mcp_need(&entry.name, &agent, &view.command, &view.args) else {
                continue;
            };
            let dep = |bins: Vec<String>| Dependent {
                kind: "mcp",
                name: entry.name.clone(),
                agents: vec![agent.clone()],
                source: "mcp",
                bins,
                any: false,
            };
            match need {
                McpNeed::Runner(id, runner) => {
                    if scope.assistant_program(&id) {
                        continue;
                    }
                    let program = programs.entry(id.clone()).or_insert_with(|| Program {
                        id,
                        ..Default::default()
                    });
                    // 同一服务在几个账号里各有一份：记一次，账号并进依赖方
                    if !program
                        .runners
                        .iter()
                        .any(|r| r.server == runner.server && r.version == runner.version)
                    {
                        program.runners.push(runner);
                    }
                    if let Some(existing) = program
                        .dependents
                        .iter_mut()
                        .find(|d| d.kind == "mcp" && d.name == entry.name)
                    {
                        if !existing.agents.contains(&agent) {
                            existing.agents.push(agent.clone());
                        }
                    } else {
                        program.dependents.push(dep(Vec::new()));
                    }
                    // 运行器本身（npx / uvx）也要在 PATH 上
                    let runner_bin = if view.command.ends_with("uvx") {
                        "uvx"
                    } else {
                        "npx"
                    };
                    let located = locate_bin(runner_bin);
                    program.located.insert(runner_bin.to_string(), located);
                }
                McpNeed::Bin(bin) => {
                    let d = dep(vec![bin.clone()]);
                    if let Some(id) = place(&mut programs, &bin, &[], &d, &mut locate_bin) {
                        if let Some(existing) = programs.get_mut(&id).and_then(|p| {
                            p.dependents
                                .iter_mut()
                                .find(|x| x.kind == "mcp" && x.name == entry.name)
                        }) {
                            if !existing.agents.contains(&agent) {
                                existing.agents.push(agent.clone());
                            }
                        }
                    }
                }
                McpNeed::Path(path) => {
                    let located = locate::classify(ctx, Path::new(&path));
                    if located.owner == "agent" {
                        continue;
                    }
                    let bin = path.rsplit('/').next().unwrap_or(&path).to_string();
                    let id = located
                        .program
                        .clone()
                        .unwrap_or_else(|| format!("path:{}", tilde(&located.real, &ctx.home_dir)));
                    let program = programs.entry(id.clone()).or_insert_with(|| Program {
                        id,
                        ..Default::default()
                    });
                    if !program.bins.contains(&bin) {
                        program.bins.push(bin.clone());
                    }
                    let exists = Path::new(&path).exists();
                    program
                        .located
                        .insert(bin.clone(), if exists { vec![located] } else { Vec::new() });
                    if let Some(existing) = program
                        .dependents
                        .iter_mut()
                        .find(|x| x.kind == "mcp" && x.name == entry.name)
                    {
                        if !existing.agents.contains(&agent) {
                            existing.agents.push(agent.clone());
                        }
                    } else {
                        program.dependents.push(dep(vec![bin]));
                    }
                }
            }
        }
    }

    // 候选只留有用的：命令在 PATH 上找得到，或能看出包标识、且这个包还没被任何声明认出来
    suggestions.retain(|s| {
        let program = s.get("program").and_then(Value::as_str);
        let known = program.is_some_and(|id| programs.contains_key(id));
        !known && (s.get("found") == Some(&Value::Bool(true)) || program.is_some())
    });

    // 托管着、但已经没有任何技能或 MCP 需要的程序：也列出来（可清理）
    for id in managed_ids(ctx) {
        if scope.assistant_program(&id) {
            continue;
        }
        programs.entry(id.clone()).or_insert_with(|| Program {
            id,
            ..Default::default()
        });
    }

    let probes = crate::native_upstream::load_probe_cache(ctx).unwrap_or_default();
    let elements: Vec<Value> = programs
        .into_values()
        .map(|p| program_json(ctx, p, &probes, &follow_latest))
        .filter(|element| {
            let bins = element["all_bins"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str);
            scope
                .ensure_extension(element["program"].as_str().unwrap_or_default(), bins)
                .is_ok()
        })
        .collect();
    let entry_dir = policy
        .pointer("/programs/entry_dir")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(json!({
        "domain": "programs",
        "elements": elements,
        "suggestions": suggestions,
        "user_path": user_path_json(ctx, &user_path),
        "entry_dir": entry_dir,
        "allow_scripts": policy.pointer("/programs/allow_scripts").cloned().unwrap_or(json!([])),
        "links": policy.pointer("/programs/links").cloned().unwrap_or(json!({})),
        "backups": super::apply::backups(ctx),
    }))
}

fn user_path_json(ctx: &Context, path: &UserPath) -> Value {
    let home = &ctx.home_dir;
    // 入口目录候选：三种 shell 形态下都在、用户可写、在 HOME 下、不是系统或 Homebrew 的目录
    let candidates: Vec<String> = path
        .stable
        .iter()
        .filter(|dir| dir.starts_with(home) && dir.is_dir())
        .filter(|dir| {
            std::fs::metadata(dir)
                .map(|m| !m.permissions().readonly())
                .unwrap_or(false)
        })
        .map(|dir| tilde(dir, home))
        .collect();
    json!({
        "dirs": path.dirs.iter().map(|d| tilde(d, home)).collect::<Vec<_>>(),
        "entry_candidates": candidates,
        "error": path.error,
    })
}

fn program_json(
    ctx: &Context,
    p: Program,
    probes: &BTreeMap<String, crate::native_upstream::ProbeRecord>,
    follow_latest: &[String],
) -> Value {
    let home = &ctx.home_dir;
    let (scheme, name) = p.id.split_once(':').unwrap_or(("bin", p.id.as_str()));
    // 装在哪：每个命令在 PATH 上第一个位置的归属（用到的就是它）
    let runner_bins = ["npx", "uvx"];
    let mut installs: Vec<Value> = Vec::new();
    let mut problems: Vec<Value> = Vec::new();
    let mut owner: Option<&'static str> = None;
    let mut version: Option<String> = None;
    let mut all_bins: Vec<String> = p.bins.clone();
    for (bin, located) in &p.located {
        let is_runner = runner_bins.contains(&bin.as_str()) && !p.runners.is_empty();
        if is_runner {
            if located.is_empty() {
                problems.push(json!({"code": "runner-missing", "bin": bin}));
            }
            continue;
        }
        if located.len() > 1 {
            problems.push(json!({"code": "multiple", "bin": bin, "paths": located.iter().map(|l| tilde(&l.path, home)).collect::<Vec<_>>()}));
        }
        // 归属与版本看属于这个包的那一份；没有认出包的才看排在最前的
        if let Some(first) = located
            .iter()
            .find(|l| l.program.as_deref() == Some(p.id.as_str()))
            .or_else(|| located.first())
        {
            owner = owner.or(Some(first.owner));
            if version.is_none() {
                version = first.version.clone();
            }
            if first.owner == "npm-global" {
                if let Some(dir) = &first.package_dir {
                    for extra in locate::npm_bins(dir) {
                        if !all_bins.contains(&extra) {
                            all_bins.push(extra);
                        }
                    }
                }
            }
        }
        for l in located {
            let value = l.to_json(home);
            if !installs.contains(&value) {
                installs.push(value);
            }
        }
    }
    // 托管的：当前版本提供的全部命令
    if let Some(dir) = managed_dir(ctx, &p.id) {
        for bin in managed_bins(&dir, scheme) {
            if !all_bins.contains(&bin) {
                all_bins.push(bin);
            }
        }
    }
    // 需要的命令缺失：「任选其一」的依赖只要有一个在就行
    let missing_bins: Vec<&String> = p
        .bins
        .iter()
        .filter(|bin| p.located.get(*bin).is_some_and(Vec::is_empty))
        .filter(|bin| {
            !p.dependents
                .iter()
                .filter(|d| d.bins.contains(bin))
                .all(|d| {
                    d.any
                        && d.bins
                            .iter()
                            .any(|other| p.located.get(other).is_some_and(|l| !l.is_empty()))
                })
        })
        .collect();
    if !missing_bins.is_empty() {
        let installable = !p.install.is_empty()
            || matches!(
                scheme,
                "npm" | "pypi" | "go" | "brew" | "github" | "download"
            );
        problems.push(json!({"code": if installable { "missing" } else { "missing-no-install" }, "bins": missing_bins}));
    }
    for runner in &p.runners {
        if !runner.pinned && !follow_latest.contains(&runner.server) {
            problems.push(json!({"code": "unpinned", "server": runner.server, "agent": runner.agent, "version": runner.version}));
        }
    }
    let mut version = version;
    let mode = if p.runners.len() == p.dependents.iter().filter(|d| d.kind == "mcp").count()
        && !p.runners.is_empty()
        && p.bins.is_empty()
    {
        "runner"
    } else {
        match owner {
            Some("managed") => "managed",
            Some("npm-global") | Some("uv-tool") => "delegated",
            Some(_) => "observed",
            None if !p.runners.is_empty() => "runner",
            None => "missing",
        }
    };
    if matches!(mode, "delegated" | "observed") && version.is_none() {
        problems.push(json!({"code": "version-unknown"}));
    }
    // 托管的版本；本工具目录里装着、却已没有依赖方的算「可清理」
    let (managed, current_managed) = managed_dir(ctx, &p.id)
        .map(|d| managed_versions(&d))
        .unwrap_or_default();
    let mode = if !managed.is_empty()
        && (mode == "missing" || mode == "runner" && p.bins.is_empty() && p.runners.is_empty())
    {
        "managed"
    } else {
        mode
    };
    if !managed.is_empty() && p.dependents.is_empty() {
        problems.push(json!({"code": "unused"}));
    }
    if mode == "managed" && version.is_none() {
        version = current_managed.clone();
    }
    let element_id = format!("programs:{}", p.id);
    // 经 npx / uvx 运行的：新版本探测沿用 MCP 服务的探测结果
    let probe_id = if probes.contains_key(&element_id) {
        element_id.clone()
    } else {
        p.runners
            .first()
            .map(|r| format!("mcp:{}", r.server))
            .unwrap_or_else(|| element_id.clone())
    };
    let runner_current = p.runners.first().and_then(|r| r.version.clone());
    let mut freshness = crate::native_upstream::freshness_from_cache(
        probes,
        &probe_id,
        version.as_deref().or(runner_current.as_deref()),
    );
    if owner == Some("uv-tool") {
        // Installed metadata is authoritative; cached current versions can be stale.
        freshness["current"] = json!(version);
        freshness["state"] = json!(match (
            version
                .as_deref()
                .and_then(|value| value.parse::<pep440_rs::Version>().ok()),
            freshness["upstream"]
                .as_str()
                .and_then(|value| value.parse::<pep440_rs::Version>().ok()),
        ) {
            (Some(current), Some(upstream)) => match current.cmp(&upstream) {
                std::cmp::Ordering::Less => "behind",
                std::cmp::Ordering::Equal => "current",
                std::cmp::Ordering::Greater => "ahead",
            },
            _ => "unknown",
        });
    }
    json!({
        "id": element_id,
        "program": p.id,
        "kind": scheme,
        "name": name,
        "bins": p.bins,
        "all_bins": all_bins,
        "mode": mode,
        "owner": owner,
        "version": version,
        "installs": installs,
        "install": p.install.iter().map(|s| json!({"kind": s.kind, "package": s.package, "bins": s.bins})).collect::<Vec<_>>(),
        "runners": p.runners.iter().map(|r| json!({"server": r.server, "agent": r.agent, "runner": r.runner, "version": r.version, "pinned": r.pinned, "follow_latest": follow_latest.contains(&r.server)})).collect::<Vec<_>>(),
        "dependents": p.dependents.iter().map(|d| json!({"kind": d.kind, "name": d.name, "agents": d.agents, "source": d.source, "bins": d.bins, "any": d.any})).collect::<Vec<_>>(),
        "problems": problems,
        "commands": manual_commands(&p.id, owner, &p.install),
        "managed_versions": managed,
        "managed_current": current_managed,
        "freshness": freshness,
    })
}

/// 用原来的方式自己处理时的命令：安装、升级、卸载（给「只观察」的程序，也给不想让本工具代办的时候）。
fn manual_commands(id: &str, owner: Option<&str>, install: &[InstallSpec]) -> Value {
    let (scheme, name) = id.split_once(':').unwrap_or(("bin", id));
    let mut out = Map::new();
    match (owner, scheme) {
        (Some("npm-global"), _) | (None, "npm") => {
            out.insert("install".into(), json!(format!("npm install -g {name}")));
            out.insert(
                "upgrade".into(),
                json!(format!("npm install -g {name}@latest")),
            );
            out.insert(
                "uninstall".into(),
                json!(format!("npm uninstall -g {name}")),
            );
        }
        (Some("uv-tool"), _) | (None, "pypi") => {
            out.insert(
                "install".into(),
                json!(format!("uv --no-python-downloads tool install {name}")),
            );
            out.insert(
                "upgrade".into(),
                json!(format!("uv --no-python-downloads tool upgrade {name}")),
            );
            out.insert(
                "uninstall".into(),
                json!(format!("uv tool uninstall {name}")),
            );
        }
        (Some("homebrew"), _) | (None, "brew") => {
            out.insert("install".into(), json!(format!("brew install {name}")));
            out.insert("upgrade".into(), json!(format!("brew upgrade {name}")));
            out.insert("uninstall".into(), json!(format!("brew uninstall {name}")));
        }
        (None, "go") => {
            out.insert("install".into(), json!(format!("go install {name}@latest")));
        }
        _ => {}
    }
    // 技能声明里给了安装方式、而上面没认出来时，照声明给
    if !out.contains_key("install") {
        if let Some(spec) = install.first() {
            let cmd = match spec.kind.as_str() {
                "node" => format!("npm install -g {}", spec.package),
                "uv" => format!("uv --no-python-downloads tool install {}", spec.package),
                "brew" => format!("brew install {}", spec.package),
                "go" => format!("go install {}@latest", spec.package),
                _ => String::new(),
            };
            if !cmd.is_empty() {
                out.insert("install".into(), json!(cmd));
            }
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copied_uv_commands_disable_runtime_downloads_in_every_status_mode() {
        for owner in [None, Some("uv-tool")] {
            let commands = manual_commands("pypi:example", owner, &[]);
            assert_eq!(
                commands["install"],
                "uv --no-python-downloads tool install example"
            );
            assert_eq!(
                commands["upgrade"],
                "uv --no-python-downloads tool upgrade example"
            );
            assert_eq!(commands["uninstall"], "uv tool uninstall example");
        }
        let commands = manual_commands(
            "bin:example",
            None,
            &[InstallSpec {
                kind: "uv".into(),
                package: "declared-package".into(),
                bins: vec!["example".into()],
            }],
        );
        assert_eq!(
            commands["install"],
            "uv --no-python-downloads tool install declared-package"
        );
    }

    #[test]
    fn managed_current_resolves_absolute_directory_links_and_rejects_external_targets() {
        let root = std::env::temp_dir().join(format!(
            "am-managed-current-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dir = root.join("program");
        let version = dir.join("1.2.0");
        std::fs::create_dir_all(&version).unwrap();
        std::fs::write(version.join(META_FILE), "{}").unwrap();
        let current = dir.join("current");
        #[cfg(windows)]
        junction::create(&version, &current).unwrap();
        #[cfg(unix)]
        crate::platform::link_dir(&version, &current).unwrap();
        assert_eq!(managed_current(&dir).unwrap(), Some("1.2.0".into()));
        assert_eq!(
            managed_versions(&dir),
            (vec!["1.2.0".into()], Some("1.2.0".into()))
        );
        crate::platform::remove_link(&current).unwrap();

        let external = root.join("outside/1.2.0");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join(META_FILE), "{}").unwrap();
        crate::platform::link_dir(&external, &current).unwrap();
        assert!(managed_current(&dir).is_err());
        assert_eq!(managed_versions(&dir).1, None);
        crate::platform::remove_link(&current).unwrap();
        crate::platform::link_dir(&dir.join("missing"), &current).unwrap();
        assert!(managed_current(&dir).is_err());
        crate::platform::remove_link(&current).unwrap();
        assert_eq!(managed_current(&dir).unwrap(), None);
        std::fs::create_dir_all(&current).unwrap();
        assert!(managed_current(&dir).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn need(command: &str, args: &[&str]) -> Option<(String, Option<String>, bool)> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        match mcp_need("s", "a", command, &args)? {
            McpNeed::Runner(id, r) => Some((id, r.version, r.pinned)),
            McpNeed::Bin(bin) => Some((format!("bin:{bin}"), None, false)),
            McpNeed::Path(p) => Some((format!("path:{p}"), None, false)),
        }
    }

    #[test]
    fn windows_路径与扩展名也认得出运行方式() {
        assert_eq!(
            need(r"C:\Program Files\nodejs\npx.CMD", &["-y", "srv@1.0.0"]),
            Some(("npm:srv".into(), Some("1.0.0".into()), true))
        );
    }

    #[test]
    fn mcp_commands_map_to_programs_and_pinning() {
        assert_eq!(
            need("npx", &["-y", "chrome-devtools-mcp@1.9.0", "--isolated"]),
            Some(("npm:chrome-devtools-mcp".into(), Some("1.9.0".into()), true))
        );
        assert_eq!(
            need("npx", &["-y", "@scope/srv"]),
            Some(("npm:@scope/srv".into(), None, false))
        );
        assert_eq!(
            need("npx", &["-y", "@scope/srv@latest"]),
            Some(("npm:@scope/srv".into(), Some("latest".into()), false))
        );
        assert_eq!(
            need(
                "uvx",
                &[
                    "--from",
                    "git+https://example.com/mcp/code@main",
                    "example-code",
                    "start"
                ]
            ),
            Some((
                "git:example.com/mcp/code".into(),
                Some("main".into()),
                false
            ))
        );
        assert_eq!(
            need("uvx", &["mcp-server-fetch==0.6.2"]),
            Some(("pypi:mcp-server-fetch".into(), Some("0.6.2".into()), true))
        );
        assert_eq!(
            need("/Applications/X.app/Contents/Resources/bin/node_repl", &[]),
            Some((
                "path:/Applications/X.app/Contents/Resources/bin/node_repl".into(),
                None,
                false
            ))
        );
        assert_eq!(
            need("wecom-cli", &["mcp"]),
            Some(("bin:wecom-cli".into(), None, false))
        );
    }
}
