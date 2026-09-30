//! 技能对程序的声明：读 SKILL.md 开头的 YAML 声明，以及从正文里找出「可能用到」的命令作为候选。
//!
//! 认两种写法（docs/PROGRAM-LIFECYCLE.md 第 6 节）：
//! - OpenClaw：`metadata.openclaw.requires.{bins, anyBins, env}` 与 `metadata.openclaw.install`
//!   （每项 `kind` 为 brew / node / go / uv / download，包名分别在 `formula` / `package` /
//!   `module` / `package` / `url`，`bins` 是它提供的命令）；`metadata` 也可能整段写成 JSON 字符串。
//! - 简写：`metadata.requires.bins`（wecom-cli 系列技能的写法）。
//!
//! 只读、不执行任何东西；声明写错时当没声明，不影响技能本身。

use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

/// 一种安装方式：`kind` 为 brew / node / go / uv / download。
#[derive(Clone, Debug, PartialEq)]
pub struct InstallSpec {
    pub kind: String,
    /// 包名（npm 包、Homebrew formula、Go 模块、PyPI 包）或下载地址。
    pub package: String,
    pub bins: Vec<String>,
}

impl InstallSpec {
    /// 程序标识：`npm:<包>`、`pypi:<包>`、`go:<模块>`、`brew:<formula>`、`download:<地址>`。
    pub fn program_id(&self) -> String {
        let scheme = match self.kind.as_str() {
            "node" => "npm",
            "uv" => "pypi",
            other => other,
        };
        format!("{scheme}:{}", self.package)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Declared {
    /// 都要有的命令。
    pub bins: Vec<String>,
    /// 任选其一即可的命令。
    pub any_bins: Vec<String>,
    pub install: Vec<InstallSpec>,
    /// Agent Skills 规范的 `compatibility`：自由文本，只原样显示。
    pub compatibility: Option<String>,
}

impl Declared {
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.bins.is_empty() && self.any_bins.is_empty() && self.install.is_empty()
    }
}

/// SKILL.md 开头 `---` 之间的那段；没有或读不出返回 None。
fn frontmatter(text: &str) -> Option<Value> {
    let rest = text.strip_prefix("---")?;
    let rest = rest
        .strip_prefix('\n')
        .or_else(|| rest.strip_prefix("\r\n"))?;
    let end = rest.find("\n---")?;
    serde_norway::from_str::<Value>(&rest[..end]).ok()
}

fn strings(value: Option<&Value>) -> Vec<String> {
    let mut out: Vec<String> = match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(one)) => vec![one.clone()],
        _ => Vec::new(),
    };
    out.retain(|name| valid_bin(name));
    out.dedup();
    out
}

/// 命令名：不能带路径分隔与空白，也不能以 `-` 开头。
pub fn valid_bin(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
}

fn install_specs(value: Option<&Value>) -> Vec<InstallSpec> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let kind = item.get("kind").and_then(Value::as_str)?.to_string();
            let key = match kind.as_str() {
                "brew" => "formula",
                "go" => "module",
                "download" => "url",
                "node" | "uv" => "package",
                _ => return None,
            };
            let package = item.get(key).and_then(Value::as_str)?.trim().to_string();
            if package.is_empty() || package.chars().any(char::is_whitespace) {
                return None;
            }
            Some(InstallSpec {
                kind,
                package,
                bins: strings(item.get("bins")),
            })
        })
        .collect()
}

/// 解析一份 SKILL.md 的程序声明。
pub fn parse_skill_md(text: &str) -> Declared {
    let Some(front) = frontmatter(text) else {
        return Declared::default();
    };
    let compatibility = front
        .get("compatibility")
        .and_then(Value::as_str)
        .map(str::to_string);
    let metadata = match front.get("metadata") {
        // OpenClaw 允许把 metadata 写成一行 JSON
        Some(Value::String(raw)) => serde_json::from_str::<Value>(raw).unwrap_or(Value::Null),
        Some(other) => other.clone(),
        None => Value::Null,
    };
    let openclaw = metadata.get("openclaw");
    let requires = openclaw
        .and_then(|o| o.get("requires"))
        .or_else(|| metadata.get("requires"));
    let install = install_specs(openclaw.and_then(|o| o.get("install")));
    Declared {
        bins: strings(requires.and_then(|r| r.get("bins"))),
        any_bins: strings(requires.and_then(|r| r.get("anyBins"))),
        install,
        compatibility,
    }
}

/// 正文里「可能用到」的命令：只看代码块与行内代码里的检查、安装写法，给出命令名、
/// 可能的包标识与出处行。只作候选，用户确认后才记为补充声明。
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub bin: String,
    /// 从安装写法里看出的包标识（`npm:@wecom/cli` 等）；只从检查写法里看出命令时为 None。
    pub program: Option<String>,
    pub line: String,
}

fn patterns() -> &'static [(Regex, &'static str)] {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            (r"command\s+-v\s+([A-Za-z0-9_.+-]+)", "bin"),
            (r"(?:^|[\s`;(|&])which\s+([A-Za-z0-9_.+-]+)", "bin"),
            (
                r"npm\s+(?:i|install)\s+(?:-g|--global)\s+(@?[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)?)",
                "npm",
            ),
            (
                r"pnpm\s+(?:add|install)\s+-g\s+(@?[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)?)",
                "npm",
            ),
            (r"brew\s+install\s+([A-Za-z0-9_.@/+-]+)", "brew"),
            (r"uv\s+tool\s+install\s+([A-Za-z0-9_.-]+)", "pypi"),
            (r"pipx\s+install\s+([A-Za-z0-9_.-]+)", "pypi"),
            (r"go\s+install\s+([A-Za-z0-9_.\-/]+)@", "go"),
        ]
        .into_iter()
        .map(|(re, kind)| (Regex::new(re).expect("固定正则"), kind))
        .collect()
    })
}

/// 代码块与行内代码的内容，逐行带回出处。
fn code_lines(text: &str) -> Vec<(String, String)> {
    static INLINE: OnceLock<Regex> = OnceLock::new();
    let inline = INLINE.get_or_init(|| Regex::new(r"`([^`]+)`").expect("固定正则"));
    let mut out = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            out.push((line.to_string(), line.trim().to_string()));
        } else {
            for cap in inline.captures_iter(line) {
                out.push((cap[1].to_string(), line.trim().to_string()));
            }
        }
    }
    out
}

pub fn suggestions(text: &str) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();
    for (code, origin) in code_lines(text) {
        for (re, kind) in patterns() {
            for cap in re.captures_iter(&code) {
                let name = cap[1].trim_end_matches(['.', ',']).to_string();
                let (bin, program) = match *kind {
                    "bin" => (name.clone(), None),
                    // npm 包的命令名通常是去掉作用域后的包名；不确定，照这个猜并允许用户改
                    "npm" => (
                        name.rsplit('/').next().unwrap_or(&name).to_string(),
                        Some(format!("npm:{name}")),
                    ),
                    "go" => (
                        name.rsplit('/').next().unwrap_or(&name).to_string(),
                        Some(format!("go:{name}")),
                    ),
                    other => (
                        name.rsplit('/').next().unwrap_or(&name).to_string(),
                        Some(format!("{other}:{name}")),
                    ),
                };
                if !valid_bin(&bin) || out.iter().any(|s| s.bin == bin && s.program == program) {
                    continue;
                }
                let line: String = origin.chars().take(200).collect();
                out.push(Suggestion { bin, program, line });
            }
        }
    }
    // 同一个命令既有检查写法又有安装写法：只留带包标识的那条
    let with_program: Vec<String> = out
        .iter()
        .filter(|s| s.program.is_some())
        .map(|s| s.bin.clone())
        .collect();
    out.retain(|s| s.program.is_some() || !with_program.contains(&s.bin));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_form_requires_bins() {
        let text = "---\nname: wecomcli-doc-manage\ndescription: x\nmetadata:\n  requires:\n    bins: [\"wecom-cli\"]\n---\n# body\n";
        let declared = parse_skill_md(text);
        assert_eq!(declared.bins, vec!["wecom-cli"]);
        assert!(declared.install.is_empty());
    }

    #[test]
    fn openclaw_form_with_install_and_json_metadata() {
        let yaml = "---\nname: jq-tool\nmetadata:\n  openclaw:\n    requires:\n      bins: [jq]\n      anyBins: [rg, grep]\n    install:\n      - kind: brew\n        formula: jq\n        bins: [jq]\n      - kind: node\n        package: typescript\n        bins: [tsc]\n      - kind: bogus\n        package: x\n---\n";
        let declared = parse_skill_md(yaml);
        assert_eq!(declared.bins, vec!["jq"]);
        assert_eq!(declared.any_bins, vec!["rg", "grep"]);
        assert_eq!(declared.install.len(), 2);
        assert_eq!(declared.install[1].program_id(), "npm:typescript");
        let json_form =
            "---\nname: x\nmetadata: '{\"openclaw\": {\"requires\": {\"bins\": [\"gh\"]}}}'\n---\n";
        assert_eq!(parse_skill_md(json_form).bins, vec!["gh"]);
    }

    #[test]
    fn broken_or_missing_frontmatter_is_no_declaration() {
        assert!(parse_skill_md("# no frontmatter").is_empty());
        assert!(parse_skill_md("---\nname: [unclosed\n---\n").is_empty());
        let bad_bins = "---\nmetadata:\n  requires:\n    bins: [\"../evil\", \"ok-bin\"]\n---\n";
        assert_eq!(parse_skill_md(bad_bins).bins, vec!["ok-bin"]);
    }

    #[test]
    fn suggestions_only_from_code_and_prefer_install_form() {
        let text = "Which field to use is up to you.\n\n```bash\ncommand -v playwright-cli\nnpm install -g @wecom/cli\n```\nRun `command -v wecom-cli` first.\n";
        let found = suggestions(text);
        assert_eq!(
            found
                .iter()
                .map(|s| (s.bin.as_str(), s.program.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                ("playwright-cli", None),
                ("cli", Some("npm:@wecom/cli")),
                ("wecom-cli", None)
            ]
        );
        assert!(
            found.iter().all(|s| s.bin != "field"),
            "正文里的 which 不算"
        );
    }
}
