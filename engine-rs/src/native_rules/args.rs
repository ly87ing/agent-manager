//! Rules 命令的严格参数解析：不复用 Request::flag/value 的宽松语义。
//! 所有错误固定描述参数种类，不回显未知 token/值，避免正文或凭据进入
//! envelope/诊断日志。

use anyhow::{bail, Result};

use super::model;
use crate::native::Request;

pub(crate) struct RuleArgs {
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

fn parse_args(req: &Request, allow_only: bool, allow_dry_run: bool) -> Result<RuleArgs> {
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
    Ok(RuleArgs {
        only,
        dry_run,
        offline,
    })
}

/// rules:status —— 只接受可选且最多一次 --offline。
pub(crate) fn parse_status_args(req: &Request) -> Result<RuleArgs> {
    parse_args(req, false, false)
}

/// rules:plan —— --only（零或多个）+ 可选最多一次 --offline，不接受 --dry-run。
pub(crate) fn parse_plan_args(req: &Request) -> Result<RuleArgs> {
    parse_args(req, true, false)
}

/// rules:apply —— 同 plan，另接受可选最多一次 --dry-run。
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

    fn both_targets() -> Vec<String> {
        vec![
            "rules:codex_global".to_string(),
            "rules:claude_global".to_string(),
        ]
    }

    #[test]
    fn status_no_args_defaults_to_both_targets() {
        let args = parse_status_args(&req(&[])).unwrap();
        assert_eq!(args.only, both_targets());
        assert!(!args.dry_run);
        assert!(!args.offline);
    }

    #[test]
    fn status_accepts_offline_once() {
        let args = parse_status_args(&req(&["--offline"])).unwrap();
        assert!(args.offline);
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
    fn plan_no_only_defaults_to_both_targets() {
        let args = parse_plan_args(&req(&["--offline"])).unwrap();
        assert_eq!(args.only, both_targets());
        assert!(args.offline);
        assert!(!args.dry_run);
    }

    #[test]
    fn plan_accepts_single_only_and_distinct_multiple_only() {
        let args = parse_plan_args(&req(&["--only", "rules:claude_global"])).unwrap();
        assert_eq!(args.only, vec!["rules:claude_global".to_string()]);

        let args = parse_plan_args(&req(&[
            "--only",
            "rules:codex_global",
            "--only",
            "rules:claude_global",
        ]))
        .unwrap();
        assert_eq!(args.only, both_targets());
    }

    #[test]
    fn plan_reversed_only_order_is_normalized_to_model_order() {
        let args = parse_plan_args(&req(&[
            "--only",
            "rules:claude_global",
            "--only",
            "rules:codex_global",
        ]))
        .unwrap();
        assert_eq!(args.only, both_targets());
    }

    #[test]
    fn plan_rejects_dry_run_unknown_flag_and_positional() {
        assert!(parse_plan_args(&req(&["--dry-run"])).is_err());
        assert!(parse_plan_args(&req(&["--verbose"])).is_err());
        assert!(parse_plan_args(&req(&["rules:codex_global"])).is_err());
        assert!(parse_plan_args(&req(&["--offline", "--offline"])).is_err());
    }

    #[test]
    fn plan_rejects_duplicate_unknown_missing_and_flag_like_only() {
        assert!(parse_plan_args(&req(&[
            "--only",
            "rules:codex_global",
            "--only",
            "rules:codex_global",
        ]))
        .is_err());
        assert!(parse_plan_args(&req(&["--only", "rules:unknown"])).is_err());
        assert!(parse_plan_args(&req(&["--only", "rules:codex"])).is_err());
        assert!(parse_plan_args(&req(&["--only", "rules:"])).is_err());
        assert!(parse_plan_args(&req(&["--only", ""])).is_err());
        assert!(parse_plan_args(&req(&["--only"])).is_err());
        assert!(parse_plan_args(&req(&["--only", "--offline"])).is_err());
    }

    #[test]
    fn apply_accepts_only_offline_and_dry_run_matrix() {
        let args = parse_apply_args(&req(&[])).unwrap();
        assert_eq!(args.only, both_targets());
        assert!(!args.dry_run && !args.offline);

        let args = parse_apply_args(&req(&[
            "--only",
            "rules:codex_global",
            "--dry-run",
            "--offline",
        ]))
        .unwrap();
        assert_eq!(args.only, vec!["rules:codex_global".to_string()]);
        assert!(args.dry_run);
        assert!(args.offline);
    }

    #[test]
    fn apply_rejects_duplicate_dry_run_and_other_invalid_input() {
        assert!(parse_apply_args(&req(&["--dry-run", "--dry-run"])).is_err());
        assert!(parse_apply_args(&req(&["--offline", "--offline"])).is_err());
        assert!(parse_apply_args(&req(&["--only", "rules:claude"])).is_err());
        assert!(parse_apply_args(&req(&["--nope"])).is_err());
        assert!(parse_apply_args(&req(&["extra"])).is_err());
        assert!(parse_apply_args(&req(&[
            "--only",
            "rules:claude_global",
            "--only",
            "rules:claude_global",
        ]))
        .is_err());
    }
}
