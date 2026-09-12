//! Hooks 命令的严格参数解析：与 rules 域同一契约。所有错误固定描述参数
//! 种类，不回显未知 token/值。

use anyhow::{bail, Result};

use super::model;
use crate::native::Request;

pub(crate) struct HookArgs {
    pub only: Vec<String>,
    pub dry_run: bool,
    pub offline: bool,
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

fn parse_args(req: &Request, allow_only: bool, allow_dry_run: bool) -> Result<HookArgs> {
    let mut selected: Vec<String> = Vec::new();
    let mut dry_run = false;
    let mut offline = false;

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
            "--dry-run" if allow_dry_run => {
                if dry_run {
                    bail!("参数错误: --dry-run 重复");
                }
                dry_run = true;
            }
            "--offline" => {
                if offline {
                    bail!("参数错误: --offline 重复");
                }
                offline = true;
            }
            other if other.starts_with("--") => {
                bail!("参数错误: 存在不支持的选项");
            }
            _ => {
                bail!("参数错误: 不接受位置参数");
            }
        }
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
        dry_run,
        offline,
    })
}

/// hooks:status —— 只接受可选且最多一次 --offline。
pub(crate) fn parse_status_args(req: &Request) -> Result<HookArgs> {
    parse_args(req, false, false)
}

/// hooks:plan —— --only（零或多个）+ 可选最多一次 --offline，不接受 --dry-run。
pub(crate) fn parse_plan_args(req: &Request) -> Result<HookArgs> {
    parse_args(req, true, false)
}

/// hooks:apply —— 同 plan，另接受可选最多一次 --dry-run。
pub(crate) fn parse_apply_args(req: &Request) -> Result<HookArgs> {
    parse_args(req, true, true)
}
