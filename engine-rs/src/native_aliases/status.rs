//! aliases 域只读状态：生效的别名就是 ~/.zshrc 与 ~/.zprofile 受管区块里的实际行。
//!
//! 逐文件输出区块内的别名、区块外用户自己的别名（只读）与写入基准 `base`（区块正文
//! sha256，没有区块为 missing）；另在顶层输出按名合并的当前别名、来源推荐（技能来源里
//! 的 aliases/*.json，不在任何文件里的同名别名才推荐；没有来源别名时附推荐来源）与旧版本
//! 保存、尚未生效的别名。status 只读零写：不创建
//! data/home 目录、DB、表或文件；目标路径不安全、读不了、非 UTF-8 或区块 marker 不成对时
//! 降级为 unmanaged-detected + blocked_reason，不提供写入。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{json, Map, Value};

use super::legacy::{self, LegacyAlias};
use super::model::{self, AliasLine, Split, TargetSpec};
use super::sources::SourceAlias;
use crate::native::Context;
use crate::native_mcp::targets::{check_symlink_metadata, validate_target_path};

/// 一个目标文件的实际状态。
pub(crate) enum Probe {
    /// 文件不存在：可以新建。
    Missing,
    /// 文件存在、是 UTF-8 普通文件且区块 marker 合法。
    Present { bytes: Vec<u8>, split: Split },
    /// 不能安全读写。
    Blocked {
        exists: Option<bool>,
        reason: String,
    },
}

impl Probe {
    /// 拆开后的内容；文件不存在时为空；不能读时为 None。
    pub(crate) fn split(&self) -> Option<Split> {
        match self {
            Probe::Present { split, .. } => Some(split.clone()),
            Probe::Missing => Some(Split {
                block: None,
                outside: Vec::new(),
            }),
            Probe::Blocked { .. } => None,
        }
    }

    pub(crate) fn writable(&self) -> bool {
        !matches!(self, Probe::Blocked { .. })
    }
}

/// 只读探测：路径安全校验 + 读取字节 + 拆区块；绝不写文件。
pub(crate) fn probe(abs_path: &Path) -> Probe {
    if let Err(err) = validate_target_path(abs_path) {
        return Probe::Blocked {
            exists: None,
            reason: format!("{err:#}"),
        };
    }
    match check_symlink_metadata(abs_path) {
        Ok(Some(meta)) if !meta.is_file() => {
            return Probe::Blocked {
                exists: Some(true),
                reason: "目标路径不是普通文件".to_string(),
            }
        }
        Ok(Some(_)) => {}
        Ok(None) => return Probe::Missing,
        Err(err) => {
            return Probe::Blocked {
                exists: None,
                reason: format!("{err:#}"),
            }
        }
    }
    let bytes = match std::fs::read(abs_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            return Probe::Blocked {
                exists: Some(true),
                reason: format!("无法读取目标文件: {err}"),
            }
        }
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Probe::Blocked {
            exists: Some(true),
            reason: "目标文件不是合法 UTF-8".to_string(),
        };
    };
    match model::split_block(text) {
        Ok(split) => Probe::Present { bytes, split },
        Err(err) => Probe::Blocked {
            exists: Some(true),
            reason: format!("{err:#}"),
        },
    }
}

/// 一个目标的一次观察。
pub(crate) struct View {
    pub spec: TargetSpec,
    /// 展示给用户的路径（~/.zshrc 本身）。
    pub abs: PathBuf,
    /// 实际读写的文件：rc 文件是符号链接（dotfiles 仓库）时为解析后的最终文件，链接本身不动。
    pub real: PathBuf,
    pub probe: Probe,
}

impl View {
    pub(crate) fn entries(&self) -> Vec<AliasLine> {
        self.probe
            .split()
            .map(|split| split.entries())
            .unwrap_or_default()
    }

    fn external(&self) -> Vec<AliasLine> {
        self.probe
            .split()
            .map(|split| split.external())
            .unwrap_or_default()
    }
}

/// Windows 命令脚本目录：本工具的脚本还原成虚拟区块（见 cmd.rs）。
fn observe_cmd(ctx: &Context, spec: TargetSpec) -> View {
    let abs = ctx.home_dir.join(spec.target);
    let probe = match super::cmd::scan(&abs) {
        Ok(scan) if scan.ours.is_empty() && scan.others.is_empty() => Probe::Missing,
        Ok(scan) => {
            let split = super::cmd::virtual_split(&scan);
            let bytes = split
                .block
                .clone()
                .unwrap_or_default()
                .join("\n")
                .into_bytes();
            Probe::Present { bytes, split }
        }
        Err(err) => Probe::Blocked {
            exists: Some(true),
            reason: format!("{err:#}"),
        },
    };
    View {
        spec,
        real: abs.clone(),
        abs,
        probe,
    }
}

/// 观察各固定目标。
pub(crate) fn observe(ctx: &Context) -> Vec<View> {
    model::targets(&ctx.home_dir)
        .into_iter()
        .map(|spec| {
            if spec.target_id == model::TARGET_CMD.target_id {
                return observe_cmd(ctx, spec);
            }
            let abs = ctx.home_dir.join(spec.target);
            // 与接入配置一致：链接解析到最终普通文件后读写；断链、指向目录等拒绝
            let (real, probe) = match crate::native_agent_settings::resolve_write_target(&abs) {
                Ok(real) => {
                    let probe = probe(&real);
                    (real, probe)
                }
                Err(err) => (
                    abs.clone(),
                    Probe::Blocked {
                        exists: Some(true),
                        reason: format!("{err:#}"),
                    },
                ),
            };
            View {
                spec,
                abs,
                real,
                probe,
            }
        })
        .collect()
}

/// 实际区块里是否已有同名同值的别名（任一可读文件）。
pub(crate) fn is_present(views: &[View], name: &str, value: &str) -> bool {
    views.iter().any(|view| {
        view.entries()
            .iter()
            .any(|e| e.name == name && e.value.as_deref() == Some(value))
    })
}

/// 来源推荐：任一文件里（区块内或区块外）已有同名别名的不推荐。
pub(crate) fn recommendations<'a>(
    views: &[View],
    source: &'a [SourceAlias],
) -> Vec<&'a SourceAlias> {
    let taken = |name: &str| {
        views.iter().any(|view| {
            view.entries()
                .iter()
                .chain(view.external().iter())
                .any(|e| e.name == name)
        })
    };
    source.iter().filter(|alias| !taken(&alias.name)).collect()
}

/// 旧版本保存、实际区块里还没有（同名同值）的别名。
pub(crate) fn legacy_suggestions(
    views: &[View],
    row: Option<&Map<String, Value>>,
) -> Vec<LegacyAlias> {
    legacy::candidates(row)
        .into_iter()
        .filter(|c| !is_present(views, &c.name, &c.value))
        .collect()
}

fn line_json(line: &AliasLine) -> Value {
    json!({ "name": line.name, "value": line.value, "line": line.line })
}

/// 按名合并两个文件区块里的别名：取值取第一个出现的文件，另给出现在哪些文件、各文件是否一致。
fn merged(views: &[View]) -> Vec<Value> {
    let mut by_name: BTreeMap<String, (AliasLine, Vec<&'static str>, bool)> = BTreeMap::new();
    for view in views {
        for entry in view.entries() {
            match by_name.get_mut(&entry.name) {
                Some((first, targets, consistent)) => {
                    targets.push(view.spec.target_id);
                    if first.line != entry.line {
                        *consistent = false;
                    }
                }
                None => {
                    by_name.insert(entry.name.clone(), (entry, vec![view.spec.target_id], true));
                }
            }
        }
    }
    by_name
        .into_values()
        .map(|(line, targets, consistent)| {
            json!({
                "name": line.name,
                "value": line.value,
                "line": line.line,
                "targets": targets,
                "consistent": consistent,
            })
        })
        .collect()
}

fn element(view: &View, on_path: Option<bool>) -> Value {
    let (state, exists, block_present, base, blocked_reason) = match &view.probe {
        Probe::Missing => (
            "missing",
            json!(false),
            json!(false),
            json!(model::BASE_MISSING),
            Value::Null,
        ),
        Probe::Present { split, .. } => (
            if split.block.is_some() {
                "managed-ok"
            } else {
                "missing"
            },
            json!(true),
            json!(split.block.is_some()),
            json!(split.base()),
            Value::Null,
        ),
        Probe::Blocked { exists, reason } => (
            "unmanaged-detected",
            exists.map_or(Value::Null, Value::Bool),
            Value::Null,
            Value::Null,
            json!(reason),
        ),
    };
    let actions = if view.probe.writable() {
        json!([{ "id": "write", "risk": "write-local", "needs_lock": true, "offline_ok": true }])
    } else {
        json!([])
    };
    json!({
        "id": view.spec.target_id,
        "domain": "aliases",
        "spec": {
            "name": view.spec.name,
            "target": format!("~/{}", view.spec.target),
            "marker": "agent-manager shell aliases",
            "base_flag": view.spec.base_flag,
            "kind": if view.spec.target_id == model::TARGET_CMD.target_id { "cmd-dir" } else { "rc-file" },
        },
        "policy": { "enabled": true },
        "observed": {
            "target": view.abs.display().to_string(),
            "target_exists": exists,
            "block_present": block_present,
            "base": base,
            "entries": view.entries().iter().map(line_json).collect::<Vec<_>>(),
            "external": view.external().iter().map(line_json).collect::<Vec<_>>(),
            "blocked_reason": blocked_reason,
            // 命令脚本目录是否已在用户 PATH 上（只对 Windows 的命令脚本目录有意义）。本工具不写
            // 注册表：不在时给出让用户自己执行的命令与手动做法
            "on_path": on_path,
            "path_command": (on_path == Some(false)).then(|| path_command(&view.abs)),
            "path_manual": (on_path == Some(false)).then(|| PATH_MANUAL.to_string()),
        },
        "state": state,
        "freshness": {
            "state": "not-applicable",
            "current": Value::Null,
            "upstream": Value::Null,
            "checked_at": Value::Null,
            "probe": "native",
            "cache_age_sec": Value::Null,
        },
        "actions": actions,
    })
}

/// 手动把命令目录加进 PATH 的做法（不想执行命令时）。
const PATH_MANUAL: &str = "按 Win+R，运行 rundll32 sysdm.cpl,EditEnvironmentVariables，在「用户变量」里编辑 Path，新建一项填这个目录，确定后重新打开终端";

/// 让用户自己在 PowerShell 里执行的命令：把命令目录追加到用户 PATH 末尾，已有时不动。
/// 按原始值读写（不展开已有条目里的 %VAR%），设、删一个临时变量让系统通知新开的终端。
fn path_command(dir: &Path) -> String {
    let dir = dir.to_string_lossy().replace('/', "\\").replace('\'', "''");
    format!(
        "$d='{dir}'; $p=(Get-Item HKCU:\\Environment).GetValue('Path','','DoNotExpandEnvironmentNames'); \
if(($p -split ';') -notcontains $d){{ Set-ItemProperty HKCU:\\Environment Path (($p.TrimEnd(';')+';'+$d).TrimStart(';')) -Type ExpandString; \
[Environment]::SetEnvironmentVariable('AM_PATH_REFRESH','1','User'); [Environment]::SetEnvironmentVariable('AM_PATH_REFRESH',$null,'User') }}"
    )
}

/// 命令脚本目录在不在用户 PATH 上（注册表里的，新开的终端用的就是它）。
pub(crate) fn cmd_dir_on_path(ctx: &Context, dir: &Path) -> bool {
    let normalize = |p: &Path| {
        p.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    };
    let wanted = normalize(dir);
    crate::native_programs::locate::user_path(ctx, false)
        .dirs
        .iter()
        .any(|p| normalize(p) == wanted)
}

pub(crate) fn build_status(ctx: &Context) -> Result<Value> {
    let views = observe(ctx);
    let row = legacy::read_row_readonly(ctx)?;
    let merged = merged(&views);
    let actual_of = |name: &str| {
        merged
            .iter()
            .find(|item| item["name"] == name)
            .map(|item| item["line"].clone())
            .unwrap_or(Value::Null)
    };
    let source = super::sources::load(ctx)?;
    let recommended: Vec<Value> = recommendations(&views, &source)
        .iter()
        .map(|alias| {
            json!({
                "name": alias.name,
                "value": alias.value,
                "line": model::alias_line(&alias.name, &alias.value),
                "source": alias.collection,
            })
        })
        .collect();
    let suggested = if source.is_empty() {
        crate::native_skills::suggested_sources(ctx)?
    } else {
        json!([])
    };
    let legacy: Vec<Value> = legacy_suggestions(&views, row.as_ref())
        .iter()
        .map(|item| {
            json!({
                "id": item.id,
                "name": item.name,
                "value": item.value,
                "line": model::alias_line(&item.name, &item.value),
                "actual": actual_of(&item.name),
            })
        })
        .collect();
    Ok(json!({
        "domain": "aliases",
        "elements": views
            .iter()
            .map(|view| {
                let on_path = (view.spec.target_id == model::TARGET_CMD.target_id)
                    .then(|| cmd_dir_on_path(ctx, &view.abs));
                element(view, on_path)
            })
            .collect::<Vec<_>>(),
        "aliases": merged,
        "alias_recommendations": recommended,
        "alias_suggested_sources": suggested,
        "alias_legacy": legacy,
    }))
}
