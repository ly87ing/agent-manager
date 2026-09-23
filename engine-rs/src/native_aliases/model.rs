//! aliases 域固定目标、受管区块的解析与编辑、旧来源结构校验与旧版内置别名（只供旧库判定）。
//!
//! 生效的别名就是 ~/.zshrc 与 ~/.zprofile 里 marker 区块内的实际行；区块外字节逐字节
//! 保留，其中用户自己的 `alias` 行只读列出。本工具写出的行固定为 `alias <name>='<value>'`；
//! 区块内其他写法的行照读照留，只在按名编辑或删除该别名时被替换。校验错误使用固定或
//! 仅含字段类别的消息，不回显正文。

use anyhow::{bail, Result};
use serde_json::{Map, Value};

use crate::native_mcp::model::sha256_hex;

pub(crate) const MARKER_BEGIN: &str = "# >>> agent-manager shell aliases >>>";
pub(crate) const MARKER_END: &str = "# <<< agent-manager shell aliases <<<";

/// 没有受管区块（文件不存在或文件里没有区块）时的写入基准。
pub(crate) const BASE_MISSING: &str = "missing";

/// 来源与旧来源都以 {timezone}/{lang} 占位，没写这两个字段时按这两个默认值解析。
const DEFAULT_TIMEZONE: &str = "America/Los_Angeles";
const DEFAULT_LANG: &str = "en_US.UTF-8";

/// 旧版本编译内置的推荐别名（默认来源），冻结在推荐别名迁出本仓（2026-09-23）时的内容，
/// 只用于判定旧库行「是否改过」；推荐内容改由技能来源提供（见 sources.rs）。
const LEGACY_BUILTIN: &str = include_str!("../../assets/legacy-aliases/shell-aliases.json");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetSpec {
    pub target_id: &'static str,
    pub name: &'static str,
    /// HOME 相对目标路径。
    pub target: &'static str,
    /// aliases:apply 里给出这个文件写入基准的参数。
    pub base_flag: &'static str,
}

pub(crate) const TARGET_ZSHRC: TargetSpec = TargetSpec {
    target_id: "aliases:zshrc",
    name: "zshrc Aliases",
    target: ".zshrc",
    base_flag: "--zshrc-base",
};

pub(crate) const TARGET_ZPROFILE: TargetSpec = TargetSpec {
    target_id: "aliases:zprofile",
    name: "zprofile Aliases",
    target: ".zprofile",
    base_flag: "--zprofile-base",
};

/// 固定目标列表，稳定顺序：zshrc 在前，zprofile 在后。
pub(crate) fn targets() -> [TargetSpec; 2] {
    [TARGET_ZSHRC, TARGET_ZPROFILE]
}

pub(crate) fn valid_alias_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// alias 值必须能安全包进单引号：不允许单引号与换行（防空格/引号注入与
/// 区块行结构破坏）；NUL 由调用方另行拒绝。
pub(crate) fn valid_alias_text(text: &str) -> bool {
    !text.contains('\'') && !text.contains('\n') && !text.contains('\r')
}

/// 校验旧来源对象（旧库 alias_sources 与配置备份里的 aliases.shell）：只允许
/// timezone/lang/aliases/_comment 字段；timezone/lang 为可安全插值的字符串；aliases 为
/// name -> string 映射，名需符合 alias 命名规则，值需可安全单引号包裹。错误不回显具体名/值。
pub(crate) fn validate_source(obj: &Map<String, Value>) -> Result<()> {
    for key in obj.keys() {
        if !matches!(key.as_str(), "timezone" | "lang" | "aliases" | "_comment") {
            bail!("aliases.shell 来源只允许 timezone/lang/aliases/_comment 字段");
        }
    }
    for key in ["timezone", "lang"] {
        if let Some(value) = obj.get(key) {
            let text = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("aliases.shell 来源 {key} 必须是字符串"))?;
            if !valid_alias_text(text) {
                bail!("aliases.shell 来源 {key} 含非法字符（引号/换行）");
            }
        }
    }
    if let Some(value) = obj.get("_comment") {
        if !value.is_string() {
            bail!("aliases.shell 来源 _comment 必须是字符串");
        }
    }
    if let Some(value) = obj.get("aliases") {
        let map = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("aliases.shell 来源 aliases 必须是对象"))?;
        for (name, value) in map {
            if !valid_alias_name(name) {
                bail!("aliases.shell 来源存在非法 alias 名（须为 [A-Za-z_][A-Za-z0-9_-]*）");
            }
            let text = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("aliases.shell 来源 alias 值必须是字符串"))?;
            if !valid_alias_text(text) {
                bail!("aliases.shell 来源存在含非法字符（引号/换行）的 alias 值");
            }
        }
    }
    Ok(())
}

/// 来源对象里的别名按其 timezone/lang 解析占位后的取值，按名排序；来源须先过 validate_source。
pub(crate) fn source_aliases(cfg: &Map<String, Value>) -> Vec<(String, String)> {
    let timezone = cfg
        .get("timezone")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_TIMEZONE);
    let lang = cfg
        .get("lang")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_LANG);
    let mut out: Vec<(String, String)> = cfg
        .get("aliases")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(name, value)| {
                    let value = value
                        .as_str()
                        .unwrap_or("")
                        .replace("{timezone}", timezone)
                        .replace("{lang}", lang);
                    (name.clone(), value)
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 旧版内置别名（按其 timezone/lang 解析后的取值），只供旧库判定。
pub(crate) fn legacy_builtin_aliases() -> Vec<(String, String)> {
    let obj: Map<String, Value> =
        serde_json::from_str(LEGACY_BUILTIN).expect("旧版内置 aliases 是合法 JSON 对象");
    source_aliases(&obj)
}

/// 本工具写出的别名行。
pub(crate) fn alias_line(name: &str, value: &str) -> String {
    format!("alias {name}='{value}'")
}

/// 解析出的一行别名定义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AliasLine {
    pub name: String,
    /// 本工具写法（`alias name='value'`，值里没有单引号）时的取值；其他写法为 None。
    pub value: Option<String>,
    /// 去掉首尾空白后的原行。
    pub line: String,
}

/// 识别一行 `alias name=...`；名称不合法（含 `alias -g` 一类选项写法）或不是别名定义时为 None。
pub(crate) fn parse_alias_line(raw: &str) -> Option<AliasLine> {
    let line = raw.trim();
    let rest = line.strip_prefix("alias ")?.trim_start();
    let (name, rhs) = rest.split_once('=')?;
    let name = name.trim();
    if !valid_alias_name(name) {
        return None;
    }
    let value = rhs
        .strip_prefix('\'')
        .and_then(|v| v.strip_suffix('\''))
        .filter(|v| !v.contains('\''))
        .map(str::to_string);
    Some(AliasLine {
        name: name.to_string(),
        value,
        line: line.to_string(),
    })
}

/// 按名去重的别名（后出现的定义生效，位置取第一次出现处）。
fn collect_aliases<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<AliasLine> {
    let mut out: Vec<AliasLine> = Vec::new();
    for raw in lines {
        let Some(parsed) = parse_alias_line(raw) else {
            continue;
        };
        match out.iter_mut().find(|item| item.name == parsed.name) {
            Some(existing) => *existing = parsed,
            None => out.push(parsed),
        }
    }
    out
}

/// 一个 rc 文件按受管区块拆开的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Split {
    /// 区块内的行（不含两行 marker）；没有区块为 None。
    pub block: Option<Vec<String>>,
    /// 区块外的行。
    pub outside: Vec<String>,
}

impl Split {
    /// 区块内的别名。
    pub(crate) fn entries(&self) -> Vec<AliasLine> {
        collect_aliases(self.block.iter().flatten().map(String::as_str))
    }

    /// 区块外用户自己的别名（只读）。
    pub(crate) fn external(&self) -> Vec<AliasLine> {
        collect_aliases(self.outside.iter().map(String::as_str))
    }

    /// 写入基准：区块正文的 sha256；没有区块为 missing。
    pub(crate) fn base(&self) -> String {
        match &self.block {
            Some(lines) => sha256_hex(&lines.join("\n")),
            None => BASE_MISSING.to_string(),
        }
    }
}

/// 拆出受管区块。起止 marker 必须恰好成对出现零次或一次，否则无法判断哪一段归本工具，
/// 拒绝（不猜测）。
pub(crate) fn split_block(text: &str) -> Result<Split> {
    let mut block: Option<Vec<String>> = None;
    let mut outside = Vec::new();
    let mut inside = false;
    for line in text.split_inclusive('\n') {
        let bare = line.strip_suffix('\n').unwrap_or(line);
        if bare == MARKER_BEGIN {
            if inside || block.is_some() {
                bail!("{MALFORMED}");
            }
            inside = true;
            block = Some(Vec::new());
            continue;
        }
        if bare == MARKER_END {
            if !inside {
                bail!("{MALFORMED}");
            }
            inside = false;
            continue;
        }
        if inside {
            block.as_mut().expect("区块已开始").push(bare.to_string());
        } else {
            outside.push(bare.to_string());
        }
    }
    if inside {
        bail!("{MALFORMED}");
    }
    Ok(Split { block, outside })
}

const MALFORMED: &str =
    "受管区块的起止注释不成对或出现多次，无法安全编辑；请在文件里手动修好后再试";

/// 对区块的一次编辑。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Edit {
    /// 新增或改写一条；`rename_from` 给出时同时去掉旧名那一条（位置沿用先出现的那条）。
    Set {
        name: String,
        value: String,
        rename_from: Option<String>,
    },
    Remove {
        name: String,
    },
}

/// 在区块行上做编辑：只替换/删除定义了相关名称的行，其余行（注释、其他写法）原样保留。
pub(crate) fn edit_lines(lines: &[String], edit: &Edit) -> Vec<String> {
    let defines = |line: &str, name: &str| parse_alias_line(line).is_some_and(|p| p.name == name);
    match edit {
        Edit::Remove { name } => lines
            .iter()
            .filter(|line| !defines(line, name))
            .cloned()
            .collect(),
        Edit::Set {
            name,
            value,
            rename_from,
        } => {
            let hit = |line: &str| {
                defines(line, name) || rename_from.as_deref().is_some_and(|old| defines(line, old))
            };
            let new_line = alias_line(name, value);
            let mut out = Vec::with_capacity(lines.len() + 1);
            let mut placed = false;
            for line in lines {
                if hit(line) {
                    if !placed {
                        out.push(new_line.clone());
                        placed = true;
                    }
                    continue;
                }
                out.push(line.clone());
            }
            if !placed {
                out.push(new_line);
            }
            out
        }
    }
}

/// 新区块正文：没有任何非空行时为空串（整个区块连同两行 marker 移除）。
pub(crate) fn block_inner(lines: &[String]) -> String {
    if lines.iter().all(|line| line.trim().is_empty()) {
        String::new()
    } else {
        lines.join("\n")
    }
}

fn block_text(expected_inner: &str) -> String {
    format!("{MARKER_BEGIN}\n{expected_inner}\n{MARKER_END}\n")
}

/// 计算写回后的文件内容：区块存在则以新正文替换（正文为空则整块移除），
/// 不存在且正文非空则以一个空行分隔追加到文件末尾；区块外字节逐字节保留。
/// 调用方须先用 split_block 确认 marker 合法。返回 None 表示无需改动。
pub(crate) fn converge_content(old: &[u8], expected_inner: &str) -> Result<Option<Vec<u8>>> {
    let text = String::from_utf8(old.to_vec())
        .map_err(|_| anyhow::anyhow!("目标文件不是合法 UTF-8，拒绝覆盖"))?;
    let found = split_block(&text)?.block.is_some();
    let mut out = String::new();
    if found {
        let mut inside = false;
        for line in text.split_inclusive('\n') {
            let bare = line.strip_suffix('\n').unwrap_or(line);
            if bare == MARKER_BEGIN {
                inside = true;
                if !expected_inner.is_empty() {
                    out.push_str(&block_text(expected_inner));
                }
                continue;
            }
            if bare == MARKER_END {
                inside = false;
                continue;
            }
            if inside {
                continue;
            }
            out.push_str(line);
        }
    } else {
        if expected_inner.is_empty() {
            return Ok(None);
        }
        out.push_str(&text);
        if !text.is_empty() {
            if !text.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
        out.push_str(&block_text(expected_inner));
    }
    if out.as_bytes() == old {
        return Ok(None);
    }
    Ok(Some(out.into_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn source_aliases_resolve_placeholders_sorted() {
        let cfg = json!({
            "timezone": "UTC",
            "lang": "C",
            "aliases": {"ts": "TZ={timezone} LANG={lang} x", "ll": "ls -l"},
        });
        assert_eq!(
            source_aliases(cfg.as_object().unwrap()),
            vec![
                ("ll".to_string(), "ls -l".to_string()),
                ("ts".to_string(), "TZ=UTC LANG=C x".to_string())
            ]
        );
        assert!(source_aliases(&Map::new()).is_empty());
        let builtin = legacy_builtin_aliases();
        let names: Vec<&str> = builtin.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["chrome", "claude", "codex", "grok"]);
        assert!(builtin[1].1.contains("TZ=\"America/Los_Angeles\""));
    }

    #[test]
    fn parse_alias_line_reads_tool_format_and_flags_others() {
        let parsed = parse_alias_line("  alias claude='TZ=\"UTC\" claude'  ").unwrap();
        assert_eq!(parsed.name, "claude");
        assert_eq!(parsed.value.as_deref(), Some("TZ=\"UTC\" claude"));
        assert_eq!(parsed.line, "alias claude='TZ=\"UTC\" claude'");
        let other = parse_alias_line("alias ll=\"ls -l\"").unwrap();
        assert_eq!(other.name, "ll");
        assert_eq!(other.value, None);
        assert_eq!(
            parse_alias_line("alias a='x' b='y'").unwrap().value,
            None,
            "一行多个定义不是本工具写法"
        );
        for bad in [
            "# alias x='1'",
            "alias -g X='1'",
            "export FOO=1",
            "alias",
            "aliasx='1'",
        ] {
            assert!(parse_alias_line(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn source_validation_rejects_injection_without_echoing() {
        let ok = json!({"aliases": {"ll": "ls -l", "my-cmd": "echo \"hi\""}, "_comment": "x"});
        validate_source(ok.as_object().unwrap()).unwrap();
        for bad in [
            json!({"aliases": {"bad name": "x"}}),
            json!({"aliases": {"bad'name": "x"}}),
            json!({"aliases": {"-bad": "x"}}),
            json!({"aliases": {"ok": "inject'ed"}}),
            json!({"aliases": {"ok": "line1\nline2"}}),
            json!({"aliases": {"ok": 1}}),
            json!({"unknown": {}}),
            json!({"timezone": "bad'tz"}),
        ] {
            let err = validate_source(bad.as_object().unwrap()).unwrap_err();
            assert!(!format!("{err:#}").contains("bad name"), "{err:#}");
            assert!(!format!("{err:#}").contains("inject"), "{err:#}");
        }
    }

    #[test]
    fn split_block_separates_inside_outside_and_rejects_malformed_markers() {
        let text = "alias mine='1'\n# >>> agent-manager shell aliases >>>\nalias a='1'\n# note\n# <<< agent-manager shell aliases <<<\nexport X=1\n";
        let split = split_block(text).unwrap();
        assert_eq!(split.block, Some(lines(&["alias a='1'", "# note"])));
        assert_eq!(split.outside, lines(&["alias mine='1'", "export X=1"]));
        assert_eq!(split.entries().len(), 1);
        assert_eq!(split.external()[0].name, "mine");
        assert_eq!(split.base(), sha256_hex("alias a='1'\n# note"));
        assert_eq!(split_block("x\n").unwrap().base(), BASE_MISSING);
        let begin = MARKER_BEGIN;
        let end = MARKER_END;
        for bad in [
            format!("{begin}\nalias a='1'\n"),
            format!("{end}\n"),
            format!("{begin}\n{begin}\n{end}\n"),
            format!("{begin}\n{end}\n{begin}\n{end}\n"),
        ] {
            let err = split_block(&bad).unwrap_err();
            assert!(format!("{err:#}").contains("不成对"), "{bad}");
        }
    }

    #[test]
    fn edit_lines_replaces_in_place_keeps_other_lines_and_renames() {
        let block = lines(&["# 我的注释", "alias a='1'", "alias b=\"2\"", "alias a='3'"]);
        let set = edit_lines(
            &block,
            &Edit::Set {
                name: "a".into(),
                value: "new".into(),
                rename_from: None,
            },
        );
        assert_eq!(
            set,
            lines(&["# 我的注释", "alias a='new'", "alias b=\"2\""])
        );
        let renamed = edit_lines(
            &block,
            &Edit::Set {
                name: "c".into(),
                value: "2".into(),
                rename_from: Some("b".into()),
            },
        );
        assert_eq!(
            renamed,
            lines(&["# 我的注释", "alias a='1'", "alias c='2'", "alias a='3'"])
        );
        let appended = edit_lines(
            &block,
            &Edit::Set {
                name: "z".into(),
                value: "9".into(),
                rename_from: None,
            },
        );
        assert_eq!(appended.last().unwrap(), "alias z='9'");
        let removed = edit_lines(&block, &Edit::Remove { name: "a".into() });
        assert_eq!(removed, lines(&["# 我的注释", "alias b=\"2\""]));
        assert_eq!(block_inner(&lines(&["", "  "])), "");
    }

    #[test]
    fn converge_preserves_outer_bytes_and_is_idempotent() {
        let old = b"# pre\nexport PATH=/x\n# >>> agent-manager shell aliases >>>\nalias old='1'\n# <<< agent-manager shell aliases <<<\n# post\n";
        let expected = "alias a='1'\nalias b='2'";
        let new = converge_content(old, expected).unwrap().unwrap();
        let text = String::from_utf8(new.clone()).unwrap();
        assert!(text.starts_with("# pre\nexport PATH=/x\n"));
        assert!(text.ends_with("# <<< agent-manager shell aliases <<<\n# post\n"));
        assert_eq!(
            split_block(&text).unwrap().block.unwrap().join("\n"),
            expected
        );
        assert!(converge_content(&new, expected).unwrap().is_none());
    }

    #[test]
    fn converge_appends_with_blank_separator_and_removes_empty_block() {
        let new = converge_content(b"# user line\n", "alias a='1'")
            .unwrap()
            .unwrap();
        assert_eq!(
            String::from_utf8(new).unwrap(),
            "# user line\n\n# >>> agent-manager shell aliases >>>\nalias a='1'\n# <<< agent-manager shell aliases <<<\n"
        );
        let new = converge_content(b"# user line", "alias a='1'")
            .unwrap()
            .unwrap();
        assert_eq!(
            String::from_utf8(new).unwrap(),
            "# user line\n\n# >>> agent-manager shell aliases >>>\nalias a='1'\n# <<< agent-manager shell aliases <<<\n"
        );
        let old = b"pre\n# >>> agent-manager shell aliases >>>\nalias a='1'\n# <<< agent-manager shell aliases <<<\npost\n";
        let new = converge_content(old, "").unwrap().unwrap();
        assert_eq!(String::from_utf8(new).unwrap(), "pre\npost\n");
        assert!(converge_content(b"pre\n", "").unwrap().is_none());
    }
}
