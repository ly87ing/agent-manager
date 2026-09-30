//! Aliases 命令的严格参数解析：与 rules 域同一契约。解析失败固定为可 downcast 的
//! UsageError（dispatch 映射 USAGE_ERROR），消息只描述参数种类，不回显未知 token/值。
//! --dry-run、--only 与 --allow-takeover 已退役，一律作为不支持的选项拒绝。

use anyhow::Result;

use super::model::{self, valid_suggestion_id, TargetSpec, LEGACY_PREFIX};
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

fn usage(message: &str) -> anyhow::Error {
    UsageError(message.to_string()).into()
}

/// 一次写入要做什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Op {
    /// 新增或改写一条；`rename_from` 给出时同时去掉旧名那一条。
    Set {
        name: String,
        value: String,
        rename_from: Option<String>,
    },
    Remove {
        name: String,
    },
    /// 按名添加一条内置推荐。
    Recommended(String),
    /// 加入一条旧版本保存、尚未生效的别名。
    Legacy(String),
}

#[derive(Debug)]
pub(crate) struct ApplyArgs {
    /// 要写的文件与各自的写入基准（status 的 observed.base），按固定目标顺序。
    pub bases: Vec<(TargetSpec, String)>,
    pub op: Op,
}

/// 逐个取 flag；值不得缺失或像选项。
fn collect<'a>(
    req: &'a Request,
    value_flags: &[&str],
    switch_flags: &[&str],
) -> Result<Vec<(&'a str, Option<&'a str>)>> {
    let mut out: Vec<(&str, Option<&str>)> = Vec::new();
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        let flag = arg.as_str();
        if !flag.starts_with("--") {
            return Err(usage("参数错误: 不接受位置参数"));
        }
        if out.iter().any(|(seen, _)| *seen == flag) {
            return Err(usage("参数错误: 选项重复"));
        }
        if switch_flags.contains(&flag) {
            out.push((flag, None));
        } else if value_flags.contains(&flag) {
            let Some(value) = iter.next() else {
                return Err(usage("参数错误: 选项缺少取值"));
            };
            if value.starts_with("--") {
                return Err(usage("参数错误: 选项取值不能是另一个选项"));
            }
            out.push((flag, Some(value.as_str())));
        } else {
            return Err(usage("参数错误: 存在不支持的选项"));
        }
    }
    Ok(out)
}

fn get<'a>(parsed: &[(&str, Option<&'a str>)], flag: &str) -> Option<&'a str> {
    parsed
        .iter()
        .find(|(f, _)| *f == flag)
        .and_then(|(_, v)| *v)
}

fn valid_base(value: &str) -> bool {
    value == model::BASE_MISSING || crate::native_mcp::model::is_sha256_hex(value)
}

fn alias_name(value: &str, flag: &str) -> Result<String> {
    if !model::valid_alias_name(value) {
        return Err(usage(&format!(
            "参数错误: {flag} 的名称须以字母或下划线开头，只含字母、数字、下划线与连字符"
        )));
    }
    Ok(value.to_string())
}

/// aliases:status —— 只接受可选且最多一次 --offline。
pub(crate) fn parse_status_args(req: &Request) -> Result<()> {
    collect(req, &[], &["--offline"])?;
    Ok(())
}

/// aliases:apply —— 至少一个文件的写入基准（--zshrc-base / --zprofile-base），外加恰好一种
/// 操作：--set <名称> --value <取值> [--rename-from <旧名>] | --remove <名称> |
/// --recommended <名称> | --legacy <建议 id>。
pub(crate) fn parse_apply_args(req: &Request) -> Result<ApplyArgs> {
    let mut value_flags: Vec<&str> = model::targets().iter().map(|t| t.base_flag).collect();
    value_flags.extend([
        "--set",
        "--value",
        "--rename-from",
        "--remove",
        "--recommended",
        "--legacy",
    ]);
    let parsed = collect(req, &value_flags, &["--offline"])?;

    let mut bases = Vec::new();
    for spec in model::targets() {
        if let Some(base) = get(&parsed, spec.base_flag) {
            if !valid_base(base) {
                return Err(usage(
                    "参数错误: 写入基准必须是 64 位小写 sha256 或 missing",
                ));
            }
            bases.push((spec, base.to_string()));
        }
    }
    if bases.is_empty() {
        return Err(usage(
            "参数错误: 需要写入基准（--zshrc-base / --zprofile-base，Windows 为 --cmd-base；取自 aliases:status 的 observed.base）",
        ));
    }

    let mut ops = Vec::new();
    if let Some(name) = get(&parsed, "--set") {
        let Some(value) = get(&parsed, "--value") else {
            return Err(usage("参数错误: --set 需要 --value"));
        };
        let rename_from = get(&parsed, "--rename-from")
            .map(|old| alias_name(old, "--rename-from"))
            .transpose()?;
        ops.push(Op::Set {
            name: alias_name(name, "--set")?,
            value: value.to_string(),
            rename_from,
        });
    } else if get(&parsed, "--value").is_some() || get(&parsed, "--rename-from").is_some() {
        return Err(usage(
            "参数错误: --value / --rename-from 只能与 --set 一起用",
        ));
    }
    if let Some(name) = get(&parsed, "--remove") {
        ops.push(Op::Remove {
            name: alias_name(name, "--remove")?,
        });
    }
    if let Some(name) = get(&parsed, "--recommended") {
        ops.push(Op::Recommended(alias_name(name, "--recommended")?));
    }
    if let Some(id) = get(&parsed, "--legacy") {
        if !valid_suggestion_id(id, LEGACY_PREFIX) {
            return Err(usage("参数错误: --legacy 建议 id 无效"));
        }
        ops.push(Op::Legacy(id.to_string()));
    }
    if ops.len() != 1 {
        return Err(usage(
            "参数错误: --set / --remove / --recommended / --legacy 必须恰好给一个",
        ));
    }
    Ok(ApplyArgs {
        bases,
        op: ops.remove(0),
    })
}

/// aliases:discard-legacy —— 恰好一个 --legacy。
pub(crate) fn parse_discard_args(req: &Request) -> Result<String> {
    let parsed = collect(req, &["--legacy"], &[])?;
    let Some(id) = get(&parsed, "--legacy") else {
        return Err(usage("参数错误: 需要 --legacy"));
    };
    if !valid_suggestion_id(id, LEGACY_PREFIX) {
        return Err(usage("参数错误: --legacy 建议 id 无效"));
    }
    Ok(id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(args: &[&str]) -> Request {
        Request {
            command: "aliases:test".to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            run_id: "run-test".to_string(),
        }
    }

    const LEGACY: &str = "legacy-0123456789abcdef";

    #[test]
    fn status_accepts_nothing_or_single_offline() {
        assert!(parse_status_args(&req(&[])).is_ok());
        assert!(parse_status_args(&req(&["--offline"])).is_ok());
        for bad in [
            vec!["--offline", "--offline"],
            vec!["--only", "aliases:zshrc"],
            vec!["--dry-run"],
            vec!["zshrc"],
        ] {
            assert!(parse_status_args(&req(&bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    #[cfg(windows)]
    fn windows_用命令脚本目录的基准_不认_rc_文件的() {
        let args = parse_apply_args(&req(&["--cmd-base", "missing", "--remove", "ll"])).unwrap();
        assert_eq!(args.bases.len(), 1);
        assert_eq!(args.bases[0].0.target_id, "aliases:cmd");
        assert!(parse_apply_args(&req(&["--zshrc-base", "missing", "--remove", "ll"])).is_err());
    }

    #[test]
    #[cfg(unix)] // zshrc / zprofile 两个文件的参数
    fn apply_accepts_each_single_operation() {
        let base = "a".repeat(64);
        let args = parse_apply_args(&req(&[
            "--zshrc-base",
            &base,
            "--zprofile-base",
            "missing",
            "--set",
            "ll",
            "--value",
            "ls -l",
            "--rename-from",
            "l",
        ]))
        .unwrap();
        assert_eq!(args.bases.len(), 2);
        assert_eq!(args.bases[0].0.target_id, "aliases:zshrc");
        assert_eq!(
            args.op,
            Op::Set {
                name: "ll".into(),
                value: "ls -l".into(),
                rename_from: Some("l".into())
            }
        );
        let args =
            parse_apply_args(&req(&["--zprofile-base", "missing", "--remove", "ll"])).unwrap();
        assert_eq!(args.bases.len(), 1);
        assert_eq!(args.bases[0].0.target_id, "aliases:zprofile");
        assert_eq!(args.op, Op::Remove { name: "ll".into() });
        let args = parse_apply_args(&req(&[
            "--zshrc-base",
            "missing",
            "--recommended",
            "claude",
        ]))
        .unwrap();
        assert_eq!(args.op, Op::Recommended("claude".into()));
        let args =
            parse_apply_args(&req(&["--zshrc-base", "missing", "--legacy", LEGACY])).unwrap();
        assert_eq!(args.op, Op::Legacy(LEGACY.into()));
    }

    #[test]
    fn apply_rejects_missing_duplicate_conflicting_and_retired_options() {
        let zb = ["--zshrc-base", "missing"];
        for bad in [
            vec![],
            vec!["--set", "ll", "--value", "x"],
            vec![zb[0], zb[1]],
            vec![zb[0], "nothex", "--remove", "ll"],
            vec![zb[0], zb[1], "--set", "ll"],
            vec![zb[0], zb[1], "--value", "x"],
            vec![zb[0], zb[1], "--remove", "ll", "--rename-from", "x"],
            vec![zb[0], zb[1], "--remove", "ll", "--recommended", "claude"],
            vec![zb[0], zb[1], "--remove", "bad name"],
            vec![zb[0], zb[1], "--set", "-x", "--value", "1"],
            vec![zb[0], zb[1], "--legacy", "cleanup-0123456789abcdef"],
            vec![zb[0], zb[1], zb[0], zb[1], "--remove", "ll"],
            vec![zb[0], zb[1], "--remove", "ll", "--only", "aliases:zshrc"],
            vec![zb[0], zb[1], "--remove", "ll", "--allow-takeover"],
            vec![zb[0], zb[1], "--remove", "ll", "--dry-run"],
            vec![zb[0], zb[1], "--remove", "ll", "positional"],
            vec![zb[0], zb[1], "--remove"],
        ] {
            assert!(parse_apply_args(&req(&bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn errors_never_echo_values() {
        let marker = "ALIAS-ARG-MARKER-7b3d";
        let bad_name = format!("{marker} x");
        for bad in [
            vec!["--zshrc-base", marker, "--remove", "ll"],
            vec!["--zshrc-base", "missing", "--remove", &bad_name],
            vec![
                "--zshrc-base",
                "missing",
                "--set",
                "ll",
                "--value",
                "x",
                marker,
            ],
            vec!["--zshrc-base", "missing", "--legacy", marker],
            vec!["--zshrc-base", "missing", marker],
        ] {
            let err = parse_apply_args(&req(&bad)).unwrap_err();
            assert!(!format!("{err:#}").contains(marker), "{bad:?}");
            assert!(err.downcast_ref::<UsageError>().is_some());
        }
    }

    #[test]
    fn discard_args() {
        assert_eq!(
            parse_discard_args(&req(&["--legacy", LEGACY])).unwrap(),
            LEGACY
        );
        for bad in [
            vec![],
            vec!["--legacy", "cleanup-0123456789abcdef"],
            vec!["--legacy", LEGACY, "--legacy", LEGACY],
            vec!["--legacy", LEGACY, "--x"],
        ] {
            assert!(parse_discard_args(&req(&bad)).is_err(), "{bad:?}");
        }
    }
}
