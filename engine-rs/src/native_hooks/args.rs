//! Hooks 命令的严格参数解析：与 rules 域同一契约。所有错误固定描述参数种类，
//! 不回显未知 token/值，避免 hook 命令或 JSON 进入 envelope。--dry-run 与
//! --allow-takeover 已退役，按不支持的选项拒绝。

use anyhow::{bail, Result};

use super::model::{self, CLEANUP_PREFIX, LEGACY_PREFIX};
use crate::native::Request;

/// 写入目标的新 hooks 从哪里来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// 直接编辑：新的完整 hooks 对象（JSON 文本，事件 → 条目数组）。
    Value(String),
    /// 清理建议 id：移除命中内置清理规则的那条 hook。
    Cleanup(String),
    /// 旧版本保存的 hook 的 id：加入实际配置。
    Legacy(String),
}

#[derive(Debug)]
pub(crate) struct ApplyArgs {
    pub target: String,
    /// status 输出的 observed.base。
    pub base: String,
    pub source: Source,
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
            bail!("参数错误: 不接受位置参数");
        }
        if out.iter().any(|(seen, _)| *seen == flag) {
            bail!("参数错误: 选项重复");
        }
        if switch_flags.contains(&flag) {
            out.push((flag, None));
        } else if value_flags.contains(&flag) {
            let Some(value) = iter.next() else {
                bail!("参数错误: 选项缺少取值");
            };
            if value.starts_with("--") {
                bail!("参数错误: 选项取值不能是另一个选项");
            }
            out.push((flag, Some(value.as_str())));
        } else {
            bail!("参数错误: 存在不支持的选项");
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

/// hooks:status —— 只接受可选且最多一次 --offline。
pub(crate) fn parse_status_args(req: &Request) -> Result<()> {
    collect(req, &[], &["--offline"])?;
    Ok(())
}

/// hooks:apply —— 恰好一个 --only 与 --base，外加恰好一种来源：
/// --value <hooks JSON> | --cleanup <清理建议 id> | --legacy <旧版 hook id>。
pub(crate) fn parse_apply_args(req: &Request) -> Result<ApplyArgs> {
    let parsed = collect(
        req,
        &["--only", "--base", "--value", "--cleanup", "--legacy"],
        &["--offline"],
    )?;
    let Some(target) = get(&parsed, "--only") else {
        bail!("参数错误: 需要 --only 指定一个 Hooks 目标");
    };
    if !model::valid_target_id(target) {
        bail!("参数错误: --only 目标无效");
    }
    let Some(base) = get(&parsed, "--base") else {
        bail!("参数错误: 需要 --base（取自 hooks:status 的 observed.base）");
    };
    if !valid_base(base) {
        bail!("参数错误: --base 必须是 64 位小写 sha256 或 missing");
    }
    let mut sources = Vec::new();
    if let Some(value) = get(&parsed, "--value") {
        sources.push(Source::Value(value.to_string()));
    }
    if let Some(id) = get(&parsed, "--cleanup") {
        if !model::valid_suggestion_id(id, CLEANUP_PREFIX) {
            bail!("参数错误: --cleanup 建议 id 无效");
        }
        sources.push(Source::Cleanup(id.to_string()));
    }
    if let Some(id) = get(&parsed, "--legacy") {
        if !model::valid_suggestion_id(id, LEGACY_PREFIX) {
            bail!("参数错误: --legacy 建议 id 无效");
        }
        sources.push(Source::Legacy(id.to_string()));
    }
    if sources.len() != 1 {
        bail!("参数错误: --value / --cleanup / --legacy 必须恰好给一个");
    }
    Ok(ApplyArgs {
        target: target.to_string(),
        base: base.to_string(),
        source: sources.remove(0),
    })
}

/// hooks:discard-legacy —— 恰好一个 --legacy。
pub(crate) fn parse_discard_args(req: &Request) -> Result<String> {
    let parsed = collect(req, &["--legacy"], &[])?;
    let Some(id) = get(&parsed, "--legacy") else {
        bail!("参数错误: 需要 --legacy");
    };
    if !model::valid_suggestion_id(id, LEGACY_PREFIX) {
        bail!("参数错误: --legacy 建议 id 无效");
    }
    Ok(id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(args: &[&str]) -> Request {
        Request {
            command: "hooks:test".to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            run_id: "run-test".to_string(),
        }
    }

    const CLEANUP: &str = "cleanup-0123456789abcdef";
    const LEGACY: &str = "legacy-0123456789abcdef";

    #[test]
    fn status_accepts_nothing_or_single_offline() {
        assert!(parse_status_args(&req(&[])).is_ok());
        assert!(parse_status_args(&req(&["--offline"])).is_ok());
        for bad in [
            vec!["--offline", "--offline"],
            vec!["--only", "hooks:claude"],
            vec!["--dry-run"],
            vec!["claude"],
        ] {
            assert!(parse_status_args(&req(&bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn apply_accepts_each_single_source() {
        let args = parse_apply_args(&req(&[
            "--only",
            "hooks:claude",
            "--base",
            "missing",
            "--value",
            "{}",
        ]))
        .unwrap();
        assert_eq!(args.source, Source::Value("{}".into()));
        let base = "a".repeat(64);
        let args = parse_apply_args(&req(&[
            "--only",
            "hooks:profile:cc-ly",
            "--base",
            &base,
            "--cleanup",
            CLEANUP,
        ]))
        .unwrap();
        assert_eq!(args.source, Source::Cleanup(CLEANUP.into()));
        let args = parse_apply_args(&req(&[
            "--only",
            "hooks:codex",
            "--base",
            "missing",
            "--legacy",
            LEGACY,
        ]))
        .unwrap();
        assert_eq!(args.source, Source::Legacy(LEGACY.into()));
    }

    #[test]
    fn apply_rejects_missing_duplicate_conflicting_and_retired_options() {
        let only = ["--only", "hooks:claude"];
        for bad in [
            vec![],
            vec![only[0], only[1]],
            vec![only[0], only[1], "--value", "{}"],
            vec![only[0], only[1], "--base", "nothex", "--value", "{}"],
            vec![only[0], only[1], "--base", "missing"],
            vec![
                only[0],
                only[1],
                "--base",
                "missing",
                "--value",
                "{}",
                "--cleanup",
                CLEANUP,
            ],
            vec![
                only[0], only[1], only[0], only[1], "--base", "missing", "--value", "{}",
            ],
            vec!["--only", "hooks:nope", "--base", "missing", "--value", "{}"],
            vec![only[0], only[1], "--base", "missing", "--cleanup", LEGACY],
            vec![
                only[0],
                only[1],
                "--base",
                "missing",
                "--legacy",
                "legacy-xyz",
            ],
            vec![
                only[0],
                only[1],
                "--base",
                "missing",
                "--value",
                "{}",
                "--allow-takeover",
            ],
            vec![
                only[0],
                only[1],
                "--base",
                "missing",
                "--value",
                "{}",
                "--dry-run",
            ],
            vec![
                only[0],
                only[1],
                "--base",
                "missing",
                "--value",
                "{}",
                "positional",
            ],
            vec![only[0], only[1], "--base", "missing", "--value"],
        ] {
            assert!(parse_apply_args(&req(&bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn errors_never_echo_values() {
        let marker = "HOOK-ARG-MARKER-7b3d";
        for bad in [
            vec!["--only", marker, "--base", "missing", "--value", "{}"],
            vec![
                "--only",
                "hooks:claude",
                "--base",
                "missing",
                "--value",
                "{}",
                marker,
            ],
            vec![
                "--only",
                "hooks:claude",
                "--base",
                "missing",
                "--legacy",
                marker,
            ],
        ] {
            let err = parse_apply_args(&req(&bad)).unwrap_err();
            assert!(!format!("{err:#}").contains(marker), "{bad:?}");
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
            vec!["--legacy", CLEANUP],
            vec!["--legacy", LEGACY, "--legacy", LEGACY],
            vec!["--legacy", LEGACY, "--x"],
        ] {
            assert!(parse_discard_args(&req(&bad)).is_err(), "{bad:?}");
        }
    }
}
