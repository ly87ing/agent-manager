//! Rules 命令的严格参数解析：不复用 Request::flag/value 的宽松语义。
//! 所有错误固定描述参数种类，不回显未知 token/值，避免规则正文或凭据进入
//! envelope。--dry-run 与 --allow-takeover 已退役，按不支持的选项拒绝。

use anyhow::{bail, Result};

use super::{model, templates};
use crate::native::Request;

/// 写入目标的新内容从哪里来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// 直接编辑：新的全文。
    Value(String),
    /// 已保存模板 id。
    Template(String),
    /// 内置推荐规则。
    Recommended,
    /// 另一个规则目标的实际正文。
    From(String),
}

#[derive(Debug)]
pub(crate) struct ApplyArgs {
    pub target: String,
    /// status 输出的 observed.base：文件字节 sha256，或 "missing"。
    pub base: String,
    pub source: Source,
}

#[derive(Debug)]
pub(crate) struct SaveArgs {
    /// 修改已有模板时的 (id, revision)；新建时为 None。
    pub existing: Option<(String, String)>,
    pub name: String,
    pub body: String,
}

/// 逐个取 flag；`free_value` 中的 flag 原样取下一个 token（正文可能以 `--` 开头，
/// 例如 YAML front matter 的 `---`），其余 flag 的值不得缺失或像选项。
fn collect<'a>(
    req: &'a Request,
    value_flags: &[&str],
    free_value: &[&str],
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
            if !free_value.contains(&flag) && value.starts_with("--") {
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

fn has(parsed: &[(&str, Option<&str>)], flag: &str) -> bool {
    parsed.iter().any(|(f, _)| *f == flag)
}

fn valid_base(value: &str) -> bool {
    value == "missing" || crate::native_mcp::model::is_sha256_hex(value)
}

/// rules:status —— 只接受可选且最多一次 --offline。
pub(crate) fn parse_status_args(req: &Request) -> Result<()> {
    collect(req, &[], &[], &["--offline"])?;
    Ok(())
}

/// rules:apply —— 恰好一个 --only 与 --base，外加恰好一种来源：
/// --value <全文> | --template <id> | --recommended | --from <目标>。
pub(crate) fn parse_apply_args(req: &Request) -> Result<ApplyArgs> {
    let parsed = collect(
        req,
        &["--only", "--base", "--value", "--template", "--from"],
        &["--value"],
        &["--recommended", "--offline"],
    )?;
    let Some(target) = get(&parsed, "--only") else {
        bail!("参数错误: 需要 --only 指定一个规则目标");
    };
    if !model::valid_target_id(target) {
        bail!("参数错误: --only 目标无效");
    }
    let Some(base) = get(&parsed, "--base") else {
        bail!("参数错误: 需要 --base（取自 rules:status 的 observed.base）");
    };
    if !valid_base(base) {
        bail!("参数错误: --base 必须是 64 位小写 sha256 或 missing");
    }
    let mut sources = Vec::new();
    if let Some(value) = get(&parsed, "--value") {
        sources.push(Source::Value(value.to_string()));
    }
    if let Some(id) = get(&parsed, "--template") {
        if !templates::valid_id(id) {
            bail!("参数错误: --template 模板 id 无效");
        }
        sources.push(Source::Template(id.to_string()));
    }
    if has(&parsed, "--recommended") {
        sources.push(Source::Recommended);
    }
    if let Some(from) = get(&parsed, "--from") {
        if !model::valid_target_id(from) {
            bail!("参数错误: --from 目标无效");
        }
        sources.push(Source::From(from.to_string()));
    }
    if sources.len() != 1 {
        bail!("参数错误: --value / --template / --recommended / --from 必须恰好给一个");
    }
    Ok(ApplyArgs {
        target: target.to_string(),
        base: base.to_string(),
        source: sources.remove(0),
    })
}

/// rules:save-template —— 新建：--name --value；修改：再加 --template 与 --revision。
pub(crate) fn parse_save_args(req: &Request) -> Result<SaveArgs> {
    let parsed = collect(
        req,
        &["--name", "--value", "--template", "--revision"],
        &["--value"],
        &[],
    )?;
    let (Some(name), Some(body)) = (get(&parsed, "--name"), get(&parsed, "--value")) else {
        bail!("参数错误: 需要 --name 与 --value");
    };
    let existing = match (get(&parsed, "--template"), get(&parsed, "--revision")) {
        (None, None) => None,
        (Some(id), Some(revision)) => {
            if !templates::valid_id(id) {
                bail!("参数错误: --template 模板 id 无效");
            }
            if !crate::native_mcp::model::is_sha256_hex(revision) {
                bail!("参数错误: --revision 必须是 64 位小写 sha256");
            }
            Some((id.to_string(), revision.to_string()))
        }
        _ => bail!("参数错误: 修改模板需要同时给 --template 与 --revision"),
    };
    Ok(SaveArgs {
        existing,
        name: name.to_string(),
        body: body.to_string(),
    })
}

/// rules:delete-template —— 恰好一个 --template。
pub(crate) fn parse_delete_args(req: &Request) -> Result<String> {
    let parsed = collect(req, &["--template"], &[], &[])?;
    let Some(id) = get(&parsed, "--template") else {
        bail!("参数错误: 需要 --template");
    };
    if !templates::valid_id(id) {
        bail!("参数错误: --template 模板 id 无效");
    }
    Ok(id.to_string())
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

    const BASE: &str = "missing";

    #[test]
    fn status_accepts_nothing_or_single_offline() {
        assert!(parse_status_args(&req(&[])).is_ok());
        assert!(parse_status_args(&req(&["--offline"])).is_ok());
        for bad in [
            vec!["--offline", "--offline"],
            vec!["--only", "rules:codex_global"],
            vec!["--dry-run"],
            vec!["codex"],
        ] {
            assert!(parse_status_args(&req(&bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn apply_accepts_each_single_source() {
        let args = parse_apply_args(&req(&[
            "--only",
            "rules:claude_global",
            "--base",
            BASE,
            "--value",
            "---\nfront: matter\n---\n",
        ]))
        .unwrap();
        assert_eq!(args.target, "rules:claude_global");
        assert_eq!(
            args.source,
            Source::Value("---\nfront: matter\n---\n".into())
        );

        let args = parse_apply_args(&req(&[
            "--only",
            "rules:profile:cc-ly",
            "--base",
            &"a".repeat(64),
            "--template",
            "tpl-1",
        ]))
        .unwrap();
        assert_eq!(args.source, Source::Template("tpl-1".into()));

        let args = parse_apply_args(&req(&[
            "--only",
            "rules:codex_global",
            "--base",
            BASE,
            "--recommended",
        ]))
        .unwrap();
        assert_eq!(args.source, Source::Recommended);

        let args = parse_apply_args(&req(&[
            "--only",
            "rules:codex_global",
            "--base",
            BASE,
            "--from",
            "rules:claude_global",
        ]))
        .unwrap();
        assert_eq!(args.source, Source::From("rules:claude_global".into()));
    }

    #[test]
    fn apply_rejects_missing_duplicate_conflicting_and_retired_options() {
        let only = ["--only", "rules:codex_global"];
        for bad in [
            vec![],
            vec!["--recommended"],
            vec![only[0], only[1], "--recommended"],
            vec![only[0], only[1], "--base", "nothex", "--recommended"],
            vec![only[0], only[1], "--base", BASE],
            vec![
                only[0],
                only[1],
                "--base",
                BASE,
                "--recommended",
                "--template",
                "tpl-1",
            ],
            vec![
                only[0], only[1], "--base", BASE, "--value", "a", "--value", "b",
            ],
            vec![
                only[0],
                only[1],
                only[0],
                only[1],
                "--base",
                BASE,
                "--recommended",
            ],
            vec!["--only", "rules:codex", "--base", BASE, "--recommended"],
            vec![only[0], only[1], "--base", BASE, "--template", "Bad"],
            vec![only[0], only[1], "--base", BASE, "--from", "codex"],
            vec![
                only[0],
                only[1],
                "--base",
                BASE,
                "--recommended",
                "--allow-takeover",
            ],
            vec![
                only[0],
                only[1],
                "--base",
                BASE,
                "--recommended",
                "--dry-run",
            ],
            vec![
                only[0],
                only[1],
                "--base",
                BASE,
                "--recommended",
                "positional",
            ],
            vec![only[0], only[1], "--base", "--recommended"],
            vec![only[0], only[1], "--base", BASE, "--value"],
        ] {
            assert!(parse_apply_args(&req(&bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn apply_errors_never_echo_values() {
        let marker = "RULE-ARG-MARKER-5c1e";
        for bad in [
            vec!["--only", marker, "--base", BASE, "--recommended"],
            vec!["--only", "rules:codex_global", "--base", BASE, marker],
            vec![
                "--only",
                "rules:codex_global",
                "--base",
                BASE,
                "--value",
                marker,
                "--x",
            ],
        ] {
            let err = parse_apply_args(&req(&bad)).unwrap_err();
            assert!(!format!("{err:#}").contains(marker), "{bad:?}");
        }
    }

    #[test]
    fn save_and_delete_args() {
        let args = parse_save_args(&req(&["--name", "我的", "--value", "正文\n"])).unwrap();
        assert!(args.existing.is_none());
        let rev = "b".repeat(64);
        let args = parse_save_args(&req(&[
            "--template",
            "tpl-1",
            "--revision",
            &rev,
            "--name",
            "我的",
            "--value",
            "",
        ]))
        .unwrap();
        assert_eq!(args.existing, Some(("tpl-1".into(), rev.clone())));
        for bad in [
            vec!["--name", "x"],
            vec!["--value", "x"],
            vec!["--name", "x", "--value", "y", "--template", "tpl-1"],
            vec!["--name", "x", "--value", "y", "--revision", &rev],
            vec![
                "--name",
                "x",
                "--value",
                "y",
                "--template",
                "tpl-1",
                "--revision",
                "short",
            ],
        ] {
            assert!(parse_save_args(&req(&bad)).is_err(), "{bad:?}");
        }
        assert_eq!(
            parse_delete_args(&req(&["--template", "tpl-1"])).unwrap(),
            "tpl-1"
        );
        assert!(parse_delete_args(&req(&[])).is_err());
        assert!(parse_delete_args(&req(&["--template", "Bad"])).is_err());
        assert!(parse_delete_args(&req(&["--template", "tpl-1", "--x"])).is_err());
    }
}
