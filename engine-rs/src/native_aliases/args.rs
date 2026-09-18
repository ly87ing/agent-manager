//! Aliases 命令的严格参数解析：与 hooks 域同一契约。解析失败固定为可
//! downcast 的 UsageError（dispatch 映射 USAGE_ERROR），消息只描述参数
//! 种类，不回显未知 token/值。--dry-run 已退役，一律作为不支持的选项拒绝。

use anyhow::Result;

use super::model;
use crate::native::Request;

/// 参数解析错误：dispatch/preflight 据此映射 USAGE_ERROR 而非域错误码。
#[derive(Debug)]
pub(crate) struct UsageError(pub(crate) String);

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UsageError {}

pub(crate) struct AliasArgs {
    pub only: Vec<String>,
    /// 显式接管：受管区块被外部改过时允许用本工具的内容替换该区块。
    pub allow_takeover: bool,
}

fn usage(message: &str) -> anyhow::Error {
    UsageError(message.to_string()).into()
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

fn parse_args(req: &Request, allow_only: bool, allow_takeover_flag: bool) -> Result<AliasArgs> {
    let mut selected: Vec<String> = Vec::new();
    let mut offline_seen = false;
    let mut allow_takeover = false;

    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--only" if allow_only => {
                let Some(value) = iter.next() else {
                    return Err(usage("参数错误: --only 缺少目标值"));
                };
                if value.starts_with("--") {
                    return Err(usage("参数错误: --only 的值不能是选项"));
                }
                if !valid_target_id(value) {
                    return Err(usage(
                        "参数错误: --only 目标无效（仅支持 aliases:zshrc / aliases:zprofile）",
                    ));
                }
                if selected.iter().any(|s| s == value) {
                    return Err(usage("参数错误: --only 目标重复"));
                }
                selected.push(value.clone());
            }
            "--allow-takeover" if allow_takeover_flag => {
                if allow_takeover {
                    return Err(usage("参数错误: --allow-takeover 重复"));
                }
                allow_takeover = true;
            }
            "--offline" => {
                if offline_seen {
                    return Err(usage("参数错误: --offline 重复"));
                }
                offline_seen = true;
            }
            other if other.starts_with("--") => {
                return Err(usage("参数错误: 存在不支持的选项"));
            }
            _ => {
                return Err(usage("参数错误: 不接受位置参数"));
            }
        }
    }

    // 接管会替换整个受管区块，必须逐个目标显式来。
    if allow_takeover && selected.len() != 1 {
        return Err(usage(
            "参数错误: --allow-takeover 仅允许与 --only 指定单个目标同时使用",
        ));
    }

    let only = if selected.is_empty() {
        model::targets()
            .iter()
            .map(|t| t.target_id.to_string())
            .collect()
    } else {
        normalize_only(&selected)
    };
    Ok(AliasArgs {
        only,
        allow_takeover,
    })
}

/// aliases:status —— 只接受可选且最多一次 --offline。
pub(crate) fn parse_status_args(req: &Request) -> Result<AliasArgs> {
    parse_args(req, false, false)
}

/// aliases:apply —— --only（零或多个）+ 可选最多一次 --offline；--dry-run
/// 与其余未知选项一样被拒绝。
pub(crate) fn parse_apply_args(req: &Request) -> Result<AliasArgs> {
    parse_args(req, true, true)
}
