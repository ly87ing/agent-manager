//! Windows 的启动命令：没有 ~/.zshrc 与 alias，每条命令是 `~/.agent-manager/bin/<名>.cmd`
//! 一个小脚本，这个目录加进用户 PATH 后，cmd 与 PowerShell 里直接敲名字就能用。
//!
//! 取值沿用各平台同一种写法（`CLAUDE_CONFIG_DIR="$HOME/.claude-cc" claude --flag`），生成脚本时
//! 转换：开头的 `名=值` 变成 `set "名=值"`（只在这次启动里有效），`$HOME` 变成 `%USERPROFILE%`
//! （后面的 `/` 改成 `\`），其余 `$VAR` 变成 `%VAR%`，命令本身后面接 `%*` 传参。
//!
//! 脚本第二行是本工具的标记，第三行记下原始取值。读回时按记下的取值重新生成一遍，与文件
//! 逐字一致才认作本工具写的、可以编辑；被手改过的只能删除（与 rc 文件里「不是本工具写法」的
//! 行一样）。目录里没有标记的 `.cmd` 是用户自己的，只读列出、不动。
//!
//! 查询、编辑、写前比对沿用 rc 文件那一套：把本工具的脚本逐个还原成一行 `alias 名='取值'`，
//! 拼成一个虚拟的受管区块；只有落盘这一步不同——按区块的变化逐个新建、改写或删除脚本。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};

use super::model::{self, Split};

/// 命令脚本目录（HOME 相对）。
pub(crate) const BIN_DIR: &str = ".agent-manager/bin";
const MARK: &str = "rem agent-manager launch command";
const VALUE_PREFIX: &str = "rem value: ";

/// 生成一条命令的脚本（CRLF 换行）。取值里有 cmd 的控制字符时拒绝，不猜怎么转义。
pub(crate) fn render(value: &str) -> Result<String> {
    if value
        .chars()
        .any(|c| matches!(c, '&' | '|' | '<' | '>' | '^' | '`'))
        || value.contains("$(")
    {
        bail!("Windows 的命令脚本里不能用 & | < > ^ ` 或 $( )，请改写这条命令");
    }
    let (envs, command) = split_env(value);
    if command.trim().is_empty() {
        bail!("命令里只有环境变量，没有要运行的程序");
    }
    let mut out = String::new();
    out.push_str("@echo off\r\n");
    out.push_str(MARK);
    out.push_str("\r\n");
    out.push_str(VALUE_PREFIX);
    out.push_str(&value.replace('%', "%%"));
    out.push_str("\r\n");
    out.push_str("setlocal\r\n");
    for (name, val) in envs {
        out.push_str(&format!("set \"{name}={}\"\r\n", convert(&val, true)));
    }
    out.push_str(&format!("{} %*\r\n", convert(command.trim(), false)));
    Ok(out)
}

/// 取值开头的 `名=值`（值可带双引号）与剩下的命令。
fn split_env(value: &str) -> (Vec<(String, String)>, &str) {
    let mut envs = Vec::new();
    let mut rest = value.trim_start();
    loop {
        let Some(eq) = rest.find('=') else { break };
        let name = &rest[..eq];
        let valid = name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            break;
        }
        let after = &rest[eq + 1..];
        let (val, remain) = if let Some(quoted) = after.strip_prefix('"') {
            let Some(end) = quoted.find('"') else { break };
            (&quoted[..end], &quoted[end + 1..])
        } else {
            let end = after.find(char::is_whitespace).unwrap_or(after.len());
            (&after[..end], &after[end..])
        };
        // 赋值后面必须是空白或结尾，否则不是一个完整的赋值（如 `A="x"y`）
        if !(remain.is_empty() || remain.starts_with(char::is_whitespace)) {
            break;
        }
        envs.push((name.to_string(), val.to_string()));
        rest = remain.trim_start();
    }
    (envs, rest)
}

/// 取值里的变量改成 cmd 写法；原有的 `%` 先转义成 `%%`。
fn convert(text: &str, path_like: bool) -> String {
    let escaped = text.replace('%', "%%");
    let re = regex::Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}|\$([A-Za-z_][A-Za-z0-9_]*)")
        .expect("固定正则");
    let replaced = re.replace_all(&escaped, |caps: &regex::Captures| {
        let name = caps
            .get(1)
            .or_else(|| caps.get(2))
            .map(|m| m.as_str())
            .unwrap_or("");
        if name == "HOME" {
            "%USERPROFILE%".to_string()
        } else {
            format!("%{name}%")
        }
    });
    let mut out = replaced.into_owned();
    // 以主目录开头的路径把 `/` 换成 `\`
    if path_like && out.starts_with("%USERPROFILE%/") {
        out = out.replace('/', "\\");
    }
    out
}

/// 这个脚本是不是本工具写的（第二行是标记）。
pub(crate) fn is_ours(text: &str) -> bool {
    text.lines().nth(1).map(str::trim_end) == Some(MARK)
}

/// 本工具的脚本还原出的取值；被手改过（与按记下的取值重新生成的内容不一致）时为 None。
pub(crate) fn recorded_value(text: &str) -> Option<String> {
    let line = text.lines().nth(2)?;
    let value = line.strip_prefix(VALUE_PREFIX)?.replace("%%", "%");
    (render(&value).ok()? == text).then_some(value)
}

/// 目录里的脚本：本工具写的（名称 → 字节）与用户自己的（名称）。
pub(crate) struct Scan {
    pub ours: BTreeMap<String, Vec<u8>>,
    pub others: Vec<String>,
}

/// 只读扫描命令目录；目录不存在时为空。
pub(crate) fn scan(dir: &Path) -> Result<Scan> {
    let mut ours = BTreeMap::new();
    let mut others = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Scan { ours, others }),
        Err(err) => return Err(err).with_context(|| format!("读不了 {}", dir.display())),
    };
    for entry in entries {
        let path = entry?.path();
        let is_cmd = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd"));
        if !is_cmd || !path.is_file() {
            continue;
        }
        let Some(name) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        let bytes = std::fs::read(&path).with_context(|| format!("读不了 {}", path.display()))?;
        if std::str::from_utf8(&bytes).is_ok_and(is_ours) && model::valid_alias_name(&name) {
            ours.insert(name, bytes);
        } else {
            others.push(name);
        }
    }
    others.sort();
    Ok(Scan { ours, others })
}

/// 把扫描结果还原成虚拟区块：本工具的脚本各一行 `alias 名='取值'`（手改过的写成认不出的一行，
/// 只能删除）；用户自己的脚本放在区块外，只读列出。没有本工具的脚本时没有区块。
pub(crate) fn virtual_split(scan: &Scan) -> Split {
    let block = (!scan.ours.is_empty()).then(|| {
        scan.ours
            .iter()
            .map(
                |(name, bytes)| match std::str::from_utf8(bytes).ok().and_then(recorded_value) {
                    Some(value) => model::alias_line(name, &value),
                    None => format!("alias {name}=（脚本被手动改过）"),
                },
            )
            .collect()
    });
    let outside = scan
        .others
        .iter()
        .map(|name| format!("alias {name}=（{name}.cmd，不是本工具写的）"))
        .collect();
    Split { block, outside }
}

/// 一个脚本的改动：`new` 为 None 表示删除。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Change {
    pub path: PathBuf,
    pub old: Option<Vec<u8>>,
    pub new: Option<Vec<u8>>,
}

/// 按编辑后的区块行算出要改哪些脚本：还在区块里、取值认得出的按取值生成；认不出的（手改过）
/// 原样不动；从区块里去掉的删除。
pub(crate) fn plan(dir: &Path, scan: &Scan, new_lines: &[String]) -> Result<Vec<Change>> {
    let mut wanted: BTreeMap<String, Option<String>> = BTreeMap::new();
    for line in new_lines {
        if let Some(parsed) = model::parse_alias_line(line) {
            wanted.insert(parsed.name, parsed.value);
        }
    }
    let mut changes = Vec::new();
    for (name, value) in &wanted {
        let old = scan.ours.get(name).cloned();
        let Some(value) = value else {
            continue; // 手改过的脚本：不动
        };
        let new = render(value)?.into_bytes();
        if old.as_deref() == Some(new.as_slice()) {
            continue;
        }
        if old.is_none()
            && scan
                .others
                .iter()
                .any(|other| other.eq_ignore_ascii_case(name))
        {
            bail!("{name}.cmd 已经是你自己的脚本，没有覆盖；换一个名字");
        }
        changes.push(Change {
            path: dir.join(format!("{name}.cmd")),
            old,
            new: Some(new),
        });
    }
    for (name, bytes) in &scan.ours {
        if !wanted.contains_key(name) {
            changes.push(Change {
                path: dir.join(format!("{name}.cmd")),
                old: Some(bytes.clone()),
                new: None,
            });
        }
    }
    Ok(changes)
}

/// 逐个落盘；每个文件写前核对仍是读到时的内容，被别处改过就停下。返回已做的改动，出错时
/// 由调用方交给 `undo` 恢复。
pub(crate) fn execute(changes: &[Change], done: &mut Vec<Change>) -> Result<()> {
    for change in changes {
        if let Some(parent) = change.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("建不了目录 {}", parent.display()))?;
        }
        let current = match std::fs::read(&change.path) {
            Ok(bytes) => Some(bytes),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => {
                return Err(err).with_context(|| format!("读不了 {}", change.path.display()))
            }
        };
        if current != change.old {
            bail!(
                "{} 已在外部修改（与打开时读到的内容不同），没有写入；请刷新后重新操作",
                change.path.display()
            );
        }
        match &change.new {
            Some(bytes) => {
                crate::native_mcp::rollback::atomic_write_bytes_checked(
                    &change.path,
                    bytes,
                    change.old.as_deref(),
                )?;
            }
            None => std::fs::remove_file(&change.path)
                .with_context(|| format!("删不了 {}", change.path.display()))?,
        }
        done.push(change.clone());
    }
    Ok(())
}

/// 撤回已做的改动：只在文件仍是本次写入的内容时恢复，别处又改过就保留现状并报告。
pub(crate) fn undo(done: &[Change]) -> Result<()> {
    let mut errors = Vec::new();
    for change in done.iter().rev() {
        let current = std::fs::read(&change.path).ok();
        if current != change.new {
            errors.push(format!(
                "{} 写入后又被改过，保留现状",
                change.path.display()
            ));
            continue;
        }
        let res = match &change.old {
            Some(bytes) => std::fs::write(&change.path, bytes),
            None => std::fs::remove_file(&change.path),
        };
        if let Err(err) = res {
            errors.push(format!("{}: {err}", change.path.display()));
        }
    }
    if !errors.is_empty() {
        bail!("命令脚本恢复失败: {}", errors.join("; "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 取值生成脚本_环境变量用_set_主目录换成_userprofile() {
        let text = render(r#"TZ="America/Los_Angeles" CLAUDE_CONFIG_DIR="$HOME/.claude-cc" claude --dangerously-skip-permissions"#).unwrap();
        assert_eq!(
            text,
            "@echo off\r\nrem agent-manager launch command\r\nrem value: TZ=\"America/Los_Angeles\" CLAUDE_CONFIG_DIR=\"$HOME/.claude-cc\" claude --dangerously-skip-permissions\r\nsetlocal\r\nset \"TZ=America/Los_Angeles\"\r\nset \"CLAUDE_CONFIG_DIR=%USERPROFILE%\\.claude-cc\"\r\nclaude --dangerously-skip-permissions %*\r\n"
        );
        assert!(is_ours(&text));
        assert_eq!(
            recorded_value(&text).as_deref(),
            Some(
                r#"TZ="America/Los_Angeles" CLAUDE_CONFIG_DIR="$HOME/.claude-cc" claude --dangerously-skip-permissions"#
            )
        );
    }

    #[test]
    fn 空赋值_清掉变量_百分号转义_其他变量() {
        let text =
            render(r#"CODEX_API_KEY= CODEX_HOME="${HOME}/.codex-w" codex --note 100%"#).unwrap();
        assert!(text.contains("set \"CODEX_API_KEY=\"\r\n"));
        assert!(text.contains("set \"CODEX_HOME=%USERPROFILE%\\.codex-w\"\r\n"));
        assert!(text.contains("codex --note 100%% %*\r\n"));
        assert_eq!(
            recorded_value(&text).as_deref(),
            Some(r#"CODEX_API_KEY= CODEX_HOME="${HOME}/.codex-w" codex --note 100%"#)
        );
        assert!(render("PATH_X=$APPDATA/x tool")
            .unwrap()
            .contains("set \"PATH_X=%APPDATA%/x\""));
    }

    #[test]
    fn 拒绝_cmd_控制字符与只有变量的取值() {
        assert!(render("claude && rm -rf x").is_err());
        assert!(render("claude > out.txt").is_err());
        assert!(render("A=1 B=2").is_err());
    }

    #[test]
    fn 手改过的脚本认不出取值_只能删除() {
        let mut text = render("claude").unwrap();
        text.push_str("echo extra\r\n");
        assert!(is_ours(&text));
        assert_eq!(recorded_value(&text), None);
    }

    #[test]
    fn 扫描_还原成虚拟区块_编辑后算出新建改写删除() {
        let dir = std::env::temp_dir().join(format!("am-cmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cc.cmd"), render("claude").unwrap()).unwrap();
        std::fs::write(dir.join("old.cmd"), render("codex").unwrap()).unwrap();
        std::fs::write(dir.join("mine.cmd"), "@echo off\r\necho hi\r\n").unwrap();
        let scan = scan(&dir).unwrap();
        let split = virtual_split(&scan);
        assert_eq!(
            split.block.clone().unwrap(),
            vec![
                "alias cc='claude'".to_string(),
                "alias old='codex'".to_string()
            ]
        );
        assert_eq!(split.external().len(), 1);
        assert_eq!(split.external()[0].name, "mine");

        let edited = model::edit_lines(
            &split.block.clone().unwrap(),
            &model::Edit::Set {
                name: "cc".into(),
                value: "claude --x".into(),
                rename_from: None,
            },
        );
        let edited = model::edit_lines(&edited, &model::Edit::Remove { name: "old".into() });
        let mut edited = model::edit_lines(
            &edited,
            &model::Edit::Set {
                name: "cx".into(),
                value: "codex".into(),
                rename_from: None,
            },
        );
        edited.sort();
        let changes = plan(&dir, &scan, &edited).unwrap();
        let summary: Vec<(String, bool, bool)> = changes
            .iter()
            .map(|c| {
                (
                    c.path.file_name().unwrap().to_string_lossy().into_owned(),
                    c.old.is_some(),
                    c.new.is_some(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("cc.cmd".to_string(), true, true),
                ("cx.cmd".to_string(), false, true),
                ("old.cmd".to_string(), true, false),
            ]
        );
        let mut done = Vec::new();
        execute(&changes, &mut done).unwrap();
        assert!(!dir.join("old.cmd").exists());
        assert_eq!(
            recorded_value(&std::fs::read_to_string(dir.join("cx.cmd")).unwrap()).as_deref(),
            Some("codex")
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("mine.cmd")).unwrap(),
            "@echo off\r\necho hi\r\n"
        );

        // 撤回：全部回到原样
        undo(&done).unwrap();
        assert!(dir.join("old.cmd").exists());
        assert!(!dir.join("cx.cmd").exists());
        assert_eq!(
            recorded_value(&std::fs::read_to_string(dir.join("cc.cmd")).unwrap()).as_deref(),
            Some("claude")
        );

        // 不覆盖用户自己的同名脚本
        let scan = super::scan(&dir).unwrap();
        let lines = vec!["alias mine='claude'".to_string()];
        assert!(plan(&dir, &scan, &lines)
            .unwrap_err()
            .to_string()
            .contains("你自己的脚本"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
