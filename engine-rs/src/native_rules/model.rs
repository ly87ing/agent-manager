#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetSpec {
    pub target_id: &'static str,
    pub agent_id: &'static str,
    pub adapter_key: &'static str,
    /// HOME 相对目标路径。
    pub target: &'static str,
}

pub(crate) const TARGET_CODEX: TargetSpec = TargetSpec {
    target_id: "rules:codex_global",
    agent_id: "codex",
    adapter_key: "rules.codex",
    target: ".codex/AGENTS.md",
};

pub(crate) const TARGET_CLAUDE: TargetSpec = TargetSpec {
    target_id: "rules:claude_global",
    agent_id: "claude-code",
    adapter_key: "rules.claude",
    target: ".claude/CLAUDE.md",
};

/// 固定目标列表，产品只支持 Codex 与 Claude。
pub(crate) fn targets() -> [TargetSpec; 2] {
    [TARGET_CODEX, TARGET_CLAUDE]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    pub target_id: String,
    pub applied_digest: String,
    pub shared_revision: String,
    pub adapter_revision: String,
    pub updated_at: String,
}

/// 渲染规则正文：逐行 trim_end，删除 trim 后为空的行，shared 在前 adapter
/// 在后，用换行拼接并保证末尾恰好一个换行。
pub(crate) fn render_rules(shared: &str, adapter: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for text in [shared, adapter] {
        for line in text.lines() {
            let trimmed = line.trim_end();
            if !trimmed.trim().is_empty() {
                lines.push(trimmed);
            }
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_are_fixed_in_codex_then_claude_order() {
        let list = targets();
        assert_eq!(list.len(), 2);

        let codex = &list[0];
        assert_eq!(codex.target_id, "rules:codex_global");
        assert_eq!(codex.agent_id, "codex");
        assert_eq!(codex.adapter_key, "rules.codex");
        assert_eq!(codex.target, ".codex/AGENTS.md");

        let claude = &list[1];
        assert_eq!(claude.target_id, "rules:claude_global");
        assert_eq!(claude.agent_id, "claude-code");
        assert_eq!(claude.adapter_key, "rules.claude");
        assert_eq!(claude.target, ".claude/CLAUDE.md");
    }

    #[test]
    fn render_rules_handles_crlf_trailing_ws_blank_and_leading_spaces() {
        let shared = "alpha  \r\n\r\n   \t\n  beta gamma\t\n";
        let adapter = "delta\r\n \r\nepsilon  ";
        let rendered = render_rules(shared, adapter);
        assert_eq!(rendered, "alpha\n  beta gamma\ndelta\nepsilon\n");
    }

    #[test]
    fn render_rules_empty_inputs_yield_single_newline() {
        assert_eq!(render_rules("", ""), "\n");
        assert_eq!(render_rules("  \n\t\n", ""), "\n");
        assert_eq!(render_rules("", "only\n"), "only\n");
    }

    #[test]
    fn render_rules_exact_bytes() {
        let rendered = render_rules("one\n", "two");
        assert_eq!(rendered.as_bytes(), b"one\ntwo\n");
    }
}
