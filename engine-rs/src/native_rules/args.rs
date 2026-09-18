//! Rules 命令的严格参数解析：不复用 Request::flag/value 的宽松语义。
//! 所有错误固定描述参数种类，不回显未知 token/值，避免正文或凭据进入
//! envelope/诊断日志。--dry-run 已退役，一律作为不支持的选项拒绝。

use anyhow::{bail, Result};

use super::model;
use crate::native::Request;

pub(crate) struct RuleArgs {
    pub only: Vec<String>,
    /// 显式接管：目标被外部改过时允许用本工具的内容整体替换它。
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

fn parse_args(req: &Request, allow_only: bool, allow_takeover_flag: bool) -> Result<RuleArgs> {
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

    // 接管是整文件替换，必须逐个目标显式来，不能一把全接管。
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
    Ok(RuleArgs {
        only,
        allow_takeover,
    })
}

/// rules:status —— 只接受可选且最多一次 --offline。
pub(crate) fn parse_status_args(req: &Request) -> Result<RuleArgs> {
    parse_args(req, false, false)
}

/// rules:apply —— --only（零或多个）+ 可选最多一次 --offline + 可选
/// --allow-takeover（须恰好选中一个目标）；--dry-run 与其余未知选项一样被拒绝。
pub(crate) fn parse_apply_args(req: &Request) -> Result<RuleArgs> {
    parse_args(req, true, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(args: &[&str]) -> Request {
        Request {
            command: "rules:test".to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            run_id: "run-test".to_string(),
        }
    }

    /// 不带 --only 时的默认目标全集（新增 OpenCode 后为三个）。
    fn both_targets() -> Vec<String> {
        vec![
            "rules:codex_global".to_string(),
            "rules:claude_global".to_string(),
            "rules:opencode_global".to_string(),
        ]
    }

    #[test]
    fn status_no_args_defaults_to_both_targets() {
        let args = parse_status_args(&req(&[])).unwrap();
        assert_eq!(args.only, both_targets());
    }

    #[test]
    fn status_accepts_offline_once() {
        let args = parse_status_args(&req(&["--offline"])).unwrap();
        assert_eq!(args.only, both_targets());
    }

    #[test]
    fn status_rejects_only_dry_run_positional_unknown_and_repeat() {
        assert!(parse_status_args(&req(&["--only", "rules:codex_global"])).is_err());
        assert!(parse_status_args(&req(&["--dry-run"])).is_err());
        assert!(parse_status_args(&req(&["codex"])).is_err());
        assert!(parse_status_args(&req(&["--bogus"])).is_err());
        assert!(parse_status_args(&req(&["--offline", "--offline"])).is_err());
    }

    #[test]
    fn apply_no_only_defaults_to_both_targets() {
        let args = parse_apply_args(&req(&["--offline"])).unwrap();
        assert_eq!(args.only, both_targets());
    }

    #[test]
    fn apply_accepts_single_only_and_distinct_multiple_only() {
        let args = parse_apply_args(&req(&["--only", "rules:claude_global"])).unwrap();
        assert_eq!(args.only, vec!["rules:claude_global".to_string()]);

        let args = parse_apply_args(&req(&[
            "--only",
            "rules:codex_global",
            "--only",
            "rules:claude_global",
        ]))
        .unwrap();
        assert_eq!(
            args.only,
            vec![
                "rules:codex_global".to_string(),
                "rules:claude_global".to_string()
            ]
        );
    }

    #[test]
    fn apply_reversed_only_order_is_normalized_to_model_order() {
        let args = parse_apply_args(&req(&[
            "--only",
            "rules:claude_global",
            "--only",
            "rules:codex_global",
        ]))
        .unwrap();
        assert_eq!(
            args.only,
            vec![
                "rules:codex_global".to_string(),
                "rules:claude_global".to_string()
            ]
        );
    }

    #[test]
    fn apply_rejects_dry_run_unknown_flag_and_positional() {
        assert!(parse_apply_args(&req(&["--dry-run"])).is_err());
        assert!(parse_apply_args(&req(&["--dry-run", "--dry-run"])).is_err());
        assert!(parse_apply_args(&req(&["--verbose"])).is_err());
        assert!(parse_apply_args(&req(&["rules:codex_global"])).is_err());
        assert!(parse_apply_args(&req(&["--offline", "--offline"])).is_err());
    }

    #[test]
    fn apply_rejects_duplicate_unknown_missing_and_flag_like_only() {
        assert!(parse_apply_args(&req(&[
            "--only",
            "rules:codex_global",
            "--only",
            "rules:codex_global",
        ]))
        .is_err());
        assert!(parse_apply_args(&req(&["--only", "rules:unknown"])).is_err());
        assert!(parse_apply_args(&req(&["--only", "rules:codex"])).is_err());
        assert!(parse_apply_args(&req(&["--only", "rules:"])).is_err());
        assert!(parse_apply_args(&req(&["--only", ""])).is_err());
        assert!(parse_apply_args(&req(&["--only"])).is_err());
        assert!(parse_apply_args(&req(&["--only", "--offline"])).is_err());
    }

    #[test]
    fn apply_rejects_dry_run_even_alongside_valid_only_and_offline() {
        assert!(parse_apply_args(&req(&[
            "--only",
            "rules:codex_global",
            "--dry-run",
            "--offline",
        ]))
        .is_err());
    }
}
