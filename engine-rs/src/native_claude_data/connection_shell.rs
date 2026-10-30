//! Lexical references only. Rewrites accept one standalone literal assignment.
use anyhow::{bail, Result};
use serde_json::{json, Value};

pub(super) const FILES: &[&str] = &[
    ".zshrc",
    ".zprofile",
    ".zshenv",
    ".bashrc",
    ".bash_profile",
    ".profile",
];

fn assignment() -> regex::Regex {
    regex::Regex::new(r#"^(?P<indent>\s*)(?:export\s+)?(?P<key>[A-Za-z_][A-Za-z0-9_]*)=(?P<value>'[^']*'|"(?:[^"\\$`]|\\.)*"|[A-Za-z0-9_./:@%+,?=-]*)(?:\s*;\s*)?(?:\s+#.*)?\s*$"#).unwrap()
}

fn simple_file(text: &str) -> bool {
    if text.contains("<<")
        || text.contains("$(")
        || text.contains('`')
        || text.lines().any(|line| line.trim_end().ends_with('\\'))
    {
        return false;
    }
    for line in text.lines() {
        // Without a shell parser, compound constructs cannot establish command position.
        let first = line
            .trim_start()
            .split(|ch: char| ch.is_whitespace() || ch == ';')
            .next()
            .unwrap_or("");
        if [
            "if",
            "then",
            "elif",
            "else",
            "fi",
            "for",
            "in",
            "select",
            "case",
            "esac",
            "while",
            "until",
            "do",
            "done",
            "function",
            "repeat",
            "foreach",
            "end",
            "coproc",
            "!",
            "time",
            "noglob",
            "nocorrect",
        ]
        .contains(&first)
        {
            return false;
        }
        let mut quote = None;
        let mut escaped = false;
        let mut whitespace = true;
        for ch in line.chars() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' && quote != Some('\'') {
                escaped = true;
                continue;
            }
            if quote == Some(ch) {
                quote = None;
            } else if quote.is_none() {
                if ch == '#' && whitespace {
                    break;
                }
                if matches!(ch, '\'' | '"' | '`') {
                    quote = Some(ch);
                } else if matches!(ch, '(' | ')' | '{' | '}' | '[' | ']' | '&' | '|') {
                    return false;
                }
            }
            whitespace = ch.is_whitespace();
        }
        if quote.is_some() {
            return false;
        }
    }
    true
}

pub(super) fn items(id: &str, bytes: &[u8]) -> Result<Vec<Value>> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("Unsupported startup file encoding."))?;
    let simple = simple_file(text);
    let assignment = assignment();
    let words=regex::Regex::new(r"\b(?:ANTHROPIC_[A-Za-z0-9_]+|CLAUDE_CONFIG_DIR|PROXY_MANAGED|HTTPS?_PROXY|NO_PROXY|ALL_PROXY|https?_proxy|no_proxy|all_proxy)\b").unwrap();
    let mut items = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let literal = assignment.captures(line);
        let mut seen = std::collections::BTreeSet::new();
        for found in words.find_iter(line) {
            let key = found.as_str();
            if !seen.insert(key) {
                continue;
            }
            let editable = simple
                && FILES.contains(&id)
                && super::connection_inspect::shared_key(key)
                && literal
                    .as_ref()
                    .is_some_and(|caps| caps.name("key").unwrap().as_str() == key);
            items.push(json!({"id":format!("shell:{id}:{}:{key}",index+1),"key":key,"line":index+1,"editable":editable}));
        }
    }
    Ok(items)
}

pub(super) fn remove(bytes: &[u8], line: usize, key: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("Unsupported startup file encoding."))?;
    if !simple_file(text) || !super::connection_inspect::shared_key(key) {
        bail!("This startup reference cannot be safely edited automatically.");
    }
    let mut output = Vec::new();
    let regex = assignment();
    let mut changed = false;
    for (index, part) in text.split_inclusive('\n').enumerate() {
        if index + 1 == line {
            let content = part.trim_end_matches(['\r', '\n']);
            let caps = regex
                .captures(content)
                .filter(|caps| caps.name("key").unwrap().as_str() == key)
                .ok_or_else(|| {
                    anyhow::anyhow!("The selected line is no longer one literal assignment.")
                })?;
            let start = caps.name("indent").unwrap().end();
            let end = caps.name("value").unwrap().end();
            output.extend_from_slice(part[..start].as_bytes());
            output.push(b'#');
            output.extend_from_slice(part[end..].as_bytes());
            changed = true;
        } else {
            output.extend_from_slice(part.as_bytes());
        }
    }
    if !changed {
        bail!("The selected startup line is absent.");
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_other_commands_and_refuses_dynamic_or_ambiguous_references() {
        let before=b"# YOUR_COMMENT\r\nexport PATH=/YOUR_PATH\r\n  export ANTHROPIC_API_KEY='PRIVATE_SELECTED' # keep comment\r\nexport CODEX_HOME=/YOUR_CODEX\r\n";
        let out = remove(before, 3, "ANTHROPIC_API_KEY").unwrap();
        assert_eq!(out,b"# YOUR_COMMENT\r\nexport PATH=/YOUR_PATH\r\n  # # keep comment\r\nexport CODEX_HOME=/YOUR_CODEX\r\n");
        for value in [
            b"export ANTHROPIC_API_KEY=$(other)\n".as_slice(),
            b"cat <<EOF\nANTHROPIC_API_KEY=x\nEOF\n",
            b"export ANTHROPIC_API_KEY=x; echo keep\n",
            b"echo $(\nexport ANTHROPIC_API_KEY=x\n)\n",
            b"export HTTP_PROXY=http://YOUR_PROXY\n",
        ] {
            assert!(remove(
                value,
                1,
                if value.starts_with(b"export HTTP") {
                    "HTTP_PROXY"
                } else {
                    "ANTHROPIC_API_KEY"
                }
            )
            .is_err());
        }
        let entries = items(
            ".zshrc",
            b"# ANTHROPIC_API_KEY=PRIVATE_COMMENT\nexport ANTHROPIC_API_KEY='PRIVATE_SELECTED'\n",
        )
        .unwrap();
        assert_eq!(entries[0]["editable"], false);
        assert_eq!(entries[1]["editable"], true);
        assert!(!serde_json::to_string(&entries)
            .unwrap()
            .contains("PRIVATE_"));
        for text in [
            "items=(\nANTHROPIC_API_KEY=x\n)\n",
            "((\nANTHROPIC_API_KEY=1\n))\n",
            "for item in\nANTHROPIC_API_KEY=x\ndo\n:\ndone\n",
            "if true; then\nANTHROPIC_API_KEY=x\nfi\n",
            "echo ${VALUE:-\nANTHROPIC_API_KEY=x\n}\n",
            "echo YOUR_VALUE &&\nANTHROPIC_API_KEY=x\necho YOUR_OTHER\n",
        ] {
            assert!(items(".zshrc", text.as_bytes())
                .unwrap()
                .iter()
                .all(|item| item["editable"] == false));
            assert!(remove(text.as_bytes(), 2, "ANTHROPIC_API_KEY").is_err());
        }
        assert_eq!(
            remove(
                b"alias :='echo YOUR_SIDE_EFFECT'\nexport ANTHROPIC_API_KEY=x\n",
                2,
                "ANTHROPIC_API_KEY"
            )
            .unwrap(),
            b"alias :='echo YOUR_SIDE_EFFECT'\n#\n"
        );
    }
}
