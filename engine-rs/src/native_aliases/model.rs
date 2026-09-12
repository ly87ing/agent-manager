//! aliases 域固定目标、来源结构校验与受管 marker 区块渲染语义。
//!
//! 受管面只有 ~/.zshrc 与 ~/.zprofile 内的 marker 区块；区块外字节逐字节
//! 保留。渲染复刻 sync_shell_aliases.sh：alias 按名排序、
//! `alias <name>='<value>'`、{timezone}/{lang} 占位替换。校验错误使用固定
//! 或仅含字段类别的消息，不回显来源正文。

use anyhow::{bail, Result};
use serde_json::{Map, Value};

pub(crate) const MARKER_BEGIN: &str = "# >>> agent-manager shell aliases >>>";
pub(crate) const MARKER_END: &str = "# <<< agent-manager shell aliases <<<";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetSpec {
    pub target_id: &'static str,
    pub name: &'static str,
    /// HOME 相对目标路径。
    pub target: &'static str,
}

pub(crate) const TARGET_ZSHRC: TargetSpec = TargetSpec {
    target_id: "aliases:zshrc",
    name: "zshrc Aliases",
    target: ".zshrc",
};

pub(crate) const TARGET_ZPROFILE: TargetSpec = TargetSpec {
    target_id: "aliases:zprofile",
    name: "zprofile Aliases",
    target: ".zprofile",
};

/// 固定目标列表，稳定顺序：zshrc 在前，zprofile 在后。
pub(crate) fn targets() -> [TargetSpec; 2] {
    [TARGET_ZSHRC, TARGET_ZPROFILE]
}

fn valid_alias_name(name: &str) -> bool {
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
/// 区块行结构破坏）；NUL 由 sources 层统一拒绝。
fn valid_alias_text(text: &str) -> bool {
    !text.contains('\'') && !text.contains('\n') && !text.contains('\r')
}

/// 校验 aliases.shell 来源对象：只允许 timezone/lang/aliases/_comment 字段；
/// timezone/lang 为可安全插值的字符串；aliases 为 name -> string 映射，名需
/// 符合 alias 命名规则，值需可安全单引号包裹。错误不回显具体名/值。
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

/// 渲染 alias 行：按名排序、占位替换；来源须先过 validate_source。
pub(crate) fn render_alias_lines(cfg: &Map<String, Value>) -> Vec<String> {
    let timezone = cfg
        .get("timezone")
        .and_then(Value::as_str)
        .unwrap_or("America/Los_Angeles");
    let lang = cfg
        .get("lang")
        .and_then(Value::as_str)
        .unwrap_or("en_US.UTF-8");
    let aliases = cfg
        .get("aliases")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut pairs: Vec<(&String, &Value)> = aliases.iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    pairs
        .into_iter()
        .map(|(name, value)| {
            let value = value.as_str().unwrap_or("");
            let value = value
                .replace("{timezone}", timezone)
                .replace("{lang}", lang);
            format!("alias {}='{}'", name, value)
        })
        .collect()
}

/// 提取受管区块：(是否找到, 区块内文本)；区块外内容忽略。多区块只认首个，
/// 与渲染侧替换语义一致。
pub(crate) fn extract_alias_block(text: &str) -> (bool, String) {
    let mut inside = false;
    let mut found = false;
    let mut inner = Vec::new();
    for line in text.split_inclusive('\n') {
        let bare = line.strip_suffix('\n').unwrap_or(line);
        if bare == MARKER_BEGIN {
            inside = true;
            found = true;
            continue;
        }
        if bare == MARKER_END {
            inside = false;
            continue;
        }
        if inside {
            inner.push(bare);
        }
    }
    (found, inner.join("\n"))
}

fn block_text(expected_inner: &str) -> String {
    format!("{MARKER_BEGIN}\n{expected_inner}\n{MARKER_END}\n")
}

/// 计算收敛后的新文件内容：区块存在则以期望内容替换（期望为空则整块移除），
/// 不存在且期望非空则以一个空行分隔追加到文件末尾；区块外字节逐字节保留。
/// 返回 None 表示无需改动。
pub(crate) fn converge_content(old: &[u8], expected_inner: &str) -> Result<Option<Vec<u8>>> {
    let text = String::from_utf8(old.to_vec())
        .map_err(|_| anyhow::anyhow!("目标文件不是合法 UTF-8，拒绝覆盖"))?;
    let (found, _) = extract_alias_block(&text);
    let mut out = String::new();
    if found {
        let mut inside = false;
        let mut emitted = false;
        for line in text.split_inclusive('\n') {
            let bare = line.strip_suffix('\n').unwrap_or(line);
            if bare == MARKER_BEGIN {
                inside = true;
                if !emitted && !expected_inner.is_empty() {
                    out.push_str(&block_text(expected_inner));
                    emitted = true;
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

    #[test]
    fn render_sorted_lines_with_placeholders() {
        let cfg = json!({
            "timezone": "UTC",
            "lang": "C",
            "aliases": {"ll": "ls -l", "ts": "TZ={timezone} LANG={lang} x"},
        });
        let lines = render_alias_lines(cfg.as_object().unwrap());
        assert_eq!(
            lines,
            vec!["alias ll='ls -l'", "alias ts='TZ=UTC LANG=C x'"]
        );
        assert!(render_alias_lines(&Map::new()).is_empty());
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
    fn converge_preserves_outer_bytes_and_is_idempotent() {
        let old = b"# pre\nexport PATH=/x\n# >>> agent-manager shell aliases >>>\nalias old='1'\n# <<< agent-manager shell aliases <<<\n# post\n";
        let expected = "alias a='1'\nalias b='2'";
        let new = converge_content(old, expected).unwrap().unwrap();
        let text = String::from_utf8(new.clone()).unwrap();
        assert!(text.starts_with("# pre\nexport PATH=/x\n"));
        assert!(text.ends_with("# <<< agent-manager shell aliases <<<\n# post\n"));
        let (found, inner) = extract_alias_block(&text);
        assert!(found);
        assert_eq!(inner, expected);
        // 幂等：再次收敛无改动
        assert!(converge_content(&new, expected).unwrap().is_none());
    }

    #[test]
    fn converge_appends_with_blank_separator_and_removes_empty_expected() {
        let old = b"# user line\n";
        let new = converge_content(old, "alias a='1'").unwrap().unwrap();
        assert_eq!(
            String::from_utf8(new).unwrap(),
            "# user line\n\n# >>> agent-manager shell aliases >>>\nalias a='1'\n# <<< agent-manager shell aliases <<<\n"
        );
        // 无结尾换行的文件也能安全追加
        let old = b"# user line";
        let new = converge_content(old, "alias a='1'").unwrap().unwrap();
        assert_eq!(
            String::from_utf8(new).unwrap(),
            "# user line\n\n# >>> agent-manager shell aliases >>>\nalias a='1'\n# <<< agent-manager shell aliases <<<\n"
        );
        // 期望为空：移除区块，保留外部
        let old = b"pre\n# >>> agent-manager shell aliases >>>\nalias a='1'\n# <<< agent-manager shell aliases <<<\npost\n";
        let new = converge_content(old, "").unwrap().unwrap();
        assert_eq!(String::from_utf8(new).unwrap(), "pre\npost\n");
        // 期望为空且无区块：无改动
        assert!(converge_content(b"pre\n", "").unwrap().is_none());
    }
}
