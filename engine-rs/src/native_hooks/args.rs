//! Hooks 命令的严格参数解析：与 rules 域同一契约。所有错误固定描述参数
//! 种类，不回显未知 token/值。--dry-run 已退役，一律作为不支持的选项拒绝。

use anyhow::{bail, Result};

use super::model;
use crate::native::Request;

pub(crate) struct HookArgs {
    pub only: Vec<String>,
    /// 显式接管通道：仅 apply 可携带，且必须与 --only 指定单个目标同时
    /// 使用；批量或缺省仍拒绝。缺省（无 flag）行为与此前完全一致。
    pub allow_takeover: bool,
}

fn valid_target_id(value: &str) -> bool {
    model::targets().iter().any(|t| t.target_id == value)
}

fn normalize_only(selected: &[String]) -> Vec<String> {
    model::targets()
        .iter()
        .filter(|t| selected.iter().any(|s| s == t.target_id))
        .map(|t| t.target_id.to_string())
        .collect()
}

fn parse_args(req: &Request, allow_only: bool, allow_takeover_flag: bool) -> Result<HookArgs> {
    let mut selected: Vec<String> = Vec::new();
    let mut offline_seen = false;
    let mut allow_takeover = false;

    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--only" if allow_only => {
                let Some(value) = iter.next() else {
                    bail!("参数错误: --only 缺少目标值");
                };
                if value.starts_with("--") {
                    bail!("参数错误: --only 的值不能是选项");
                }
                if !valid_target_id(value) {
                    bail!("参数错误: --only 目标无效");
                }
                if selected.iter().any(|s| s == value) {
                    bail!("参数错误: --only 目标重复");
                }
                selected.push(value.clone());
            }
            "--allow-takeover" if allow_takeover_flag => {
                if allow_takeover {
                    bail!("参数错误: --allow-takeover 重复");
                }
                allow_takeover = true;
            }
            "--offline" => {
                if offline_seen {
                    bail!("参数错误: --offline 重复");
                }
                offline_seen = true;
            }
            other if other.starts_with("--") => {
                bail!("参数错误: 存在不支持的选项");
            }
            _ => {
                bail!("参数错误: 不接受位置参数");
            }
        }
    }

    if allow_takeover && selected.len() != 1 {
        bail!("参数错误: --allow-takeover 仅允许与 --only 指定单个目标同时使用");
    }

    let only = if selected.is_empty() {
        model::targets()
            .iter()
            .map(|t| t.target_id.to_string())
            .collect()
    } else {
        normalize_only(&selected)
    };
    Ok(HookArgs {
        only,
        allow_takeover,
    })
}

/// hooks:status —— 只接受可选且最多一次 --offline。
pub(crate) fn parse_status_args(req: &Request) -> Result<HookArgs> {
    parse_args(req, false, false)
}

/// hooks:apply —— --only（零或多个）+ 可选最多一次 --offline /
/// --allow-takeover；--dry-run 与其余未知选项一样被拒绝。
pub(crate) fn parse_apply_args(req: &Request) -> Result<HookArgs> {
    parse_args(req, true, true)
}
