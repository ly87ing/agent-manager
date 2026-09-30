//! git 来源的跟踪方式与版本解析（纯逻辑，不起进程、不访问网络）。
//!
//! 来源定义里的 `track`：
//! - 缺省或 `{"mode":"auto"}`：仓库有版本号标签就跟最新版本，否则跟主分支；
//! - `{"mode":"tag","pattern":"v*","prerelease":false}`：匹配规则内版本号最大的标签；
//! - `{"mode":"default"}`：远端默认分支；
//! - `{"mode":"branch","branch":"stable"}`：指定分支；
//! - `{"mode":"branch-pattern","pattern":"release/*"}`：分支名里版本号最大的匹配分支；
//! - `{"mode":"pinned","ref":"v3.0.1"}`：固定到标签、分支或提交，不自动变。
//!
//! 远端引用来自 `git ls-remote --symref <url> HEAD refs/heads/* refs/tags/*` 的输出。

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Track {
    Auto,
    Tag { pattern: String, prerelease: bool },
    Default,
    Branch(String),
    BranchPattern(String),
    Pinned(String),
}

const DEFAULT_TAG_PATTERN: &str = "*";

/// 引用名：只允许常见字符，不能以 `-` 开头（防止被 git 当成参数）。
pub(crate) fn valid_ref_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('-')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '+' | '*'))
}

impl Track {
    /// 从来源定义的 `track` 字段解析；缺省即 auto。格式不对时报错（不静默当成 auto）。
    pub(crate) fn parse(raw: Option<&Value>) -> Result<Track> {
        let Some(raw) = raw else {
            return Ok(Track::Auto);
        };
        let obj = raw.as_object().ok_or_else(|| anyhow!("track 必须是对象"))?;
        let text = |key: &str| -> Result<String> {
            let value = obj
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| anyhow!("track.{key} 不能为空"))?;
            if !valid_ref_name(value) {
                bail!("track.{key} 含不允许的字符：{value}");
            }
            Ok(value.to_string())
        };
        match obj.get("mode").and_then(Value::as_str).unwrap_or("auto") {
            "auto" => Ok(Track::Auto),
            "tag" => Ok(Track::Tag {
                pattern: match obj.get("pattern") {
                    None | Some(Value::Null) => DEFAULT_TAG_PATTERN.to_string(),
                    Some(_) => text("pattern")?,
                },
                prerelease: obj
                    .get("prerelease")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }),
            "default" => Ok(Track::Default),
            "branch" => Ok(Track::Branch(text("branch")?)),
            "branch-pattern" => Ok(Track::BranchPattern(text("pattern")?)),
            "pinned" => {
                let value = text("ref")?;
                if value.contains('*') {
                    bail!("track.ref 不能含通配符");
                }
                Ok(Track::Pinned(value))
            }
            other => bail!("不认识的跟踪方式：{other}"),
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        match self {
            Track::Auto => json!({"mode": "auto"}),
            Track::Tag {
                pattern,
                prerelease,
            } => json!({"mode": "tag", "pattern": pattern, "prerelease": prerelease}),
            Track::Default => json!({"mode": "default"}),
            Track::Branch(branch) => json!({"mode": "branch", "branch": branch}),
            Track::BranchPattern(pattern) => json!({"mode": "branch-pattern", "pattern": pattern}),
            Track::Pinned(reference) => json!({"mode": "pinned", "ref": reference}),
        }
    }
}

/// 远端的分支与标签（标签取剥离后的提交，即 `^{}` 行）。
#[derive(Debug, Default, Clone)]
pub(crate) struct RemoteRefs {
    pub(crate) default_branch: Option<String>,
    pub(crate) heads: BTreeMap<String, String>,
    pub(crate) tags: BTreeMap<String, String>,
}

impl RemoteRefs {
    pub(crate) fn parse(output: &str) -> RemoteRefs {
        let mut refs = RemoteRefs::default();
        let mut peeled: BTreeMap<String, String> = BTreeMap::new();
        for line in output.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("ref: ") {
                // `ref: refs/heads/main\tHEAD`
                let mut parts = rest.split_whitespace();
                if let (Some(target), Some("HEAD")) = (parts.next(), parts.next()) {
                    refs.default_branch = target.strip_prefix("refs/heads/").map(str::to_string);
                }
                continue;
            }
            let mut parts = line.split_whitespace();
            let (Some(sha), Some(name)) = (parts.next(), parts.next()) else {
                continue;
            };
            if !is_sha(sha) {
                continue;
            }
            if let Some(branch) = name.strip_prefix("refs/heads/") {
                refs.heads.insert(branch.to_string(), sha.to_string());
            } else if let Some(tag) = name.strip_prefix("refs/tags/") {
                match tag.strip_suffix("^{}") {
                    Some(tag) => {
                        peeled.insert(tag.to_string(), sha.to_string());
                    }
                    None => {
                        refs.tags.entry(tag.to_string()).or_insert(sha.to_string());
                    }
                }
            }
        }
        refs.tags.extend(peeled);
        refs
    }

    /// 指向某个提交的标签里，版本号最大的那个（没有版本号的按名字取第一个）。
    pub(crate) fn tag_at(&self, commit: &str) -> Option<String> {
        let mut names: Vec<&String> = self
            .tags
            .iter()
            .filter(|(_, sha)| sha.as_str() == commit)
            .map(|(name, _)| name)
            .collect();
        names.sort_by(|a, b| match (Version::parse(b), Version::parse(a)) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => Ordering::Greater,
            (None, Some(_)) => Ordering::Less,
            (None, None) => a.cmp(b),
        });
        names.first().map(|name| name.to_string())
    }
}

pub(crate) fn is_sha(text: &str) -> bool {
    (7..=64).contains(&text.len()) && text.chars().all(|c| c.is_ascii_hexdigit())
}

/// 解析出的目标版本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    /// `tag` / `branch` / `commit`
    pub(crate) kind: &'static str,
    pub(crate) name: String,
    pub(crate) commit: String,
}

impl Target {
    /// 界面上显示的名字：标签与分支用名字，提交用前 7 位。
    pub(crate) fn label(&self) -> String {
        match self.kind {
            "commit" => self.commit.chars().take(7).collect(),
            _ => self.name.clone(),
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({"kind": self.kind, "name": self.name, "commit": self.commit, "label": self.label()})
    }
}

/// 从名字里取版本号：第一段 `数字(.数字)+` 及其后紧跟的 `-预发布`。
/// 相等与大小一致：`3.0` 与 `3.0.0` 相等。
#[derive(Debug, Clone)]
pub(crate) struct Version {
    numbers: Vec<u64>,
    pre: Option<String>,
}

impl Version {
    pub(crate) fn parse(name: &str) -> Option<Version> {
        let re = regex::Regex::new(r"(\d+(?:\.\d+)+)(?:-([0-9A-Za-z][0-9A-Za-z.-]*))?")
            .expect("静态正则合法");
        let caps = re.captures(name)?;
        let mut numbers = Vec::new();
        for part in caps.get(1)?.as_str().split('.') {
            if part.len() > 18 {
                return None;
            }
            numbers.push(part.parse::<u64>().ok()?);
        }
        Some(Version {
            numbers,
            pre: caps.get(2).map(|m| m.as_str().to_string()),
        })
    }

    pub(crate) fn is_prerelease(&self) -> bool {
        self.pre.is_some()
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let width = self.numbers.len().max(other.numbers.len());
        for i in 0..width {
            let a = self.numbers.get(i).copied().unwrap_or(0);
            let b = other.numbers.get(i).copied().unwrap_or(0);
            if a != b {
                return a.cmp(&b);
            }
        }
        match (&self.pre, &other.pre) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(a), Some(b)) => compare_pre(a, b),
        }
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 预发布段按 `.` 逐段比：都是数字按数值，否则按字符串。
fn compare_pre(a: &str, b: &str) -> Ordering {
    let mut left = a.split('.');
    let mut right = b.split('.');
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let order = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    _ => x.cmp(y),
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

/// 只支持 `*` 通配（匹配任意字符，含 `/`）。
pub(crate) fn glob_match(pattern: &str, name: &str) -> bool {
    let escaped: Vec<String> = pattern.split('*').map(regex::escape).collect();
    regex::Regex::new(&format!("^{}$", escaped.join(".*")))
        .map(|re| re.is_match(name))
        .unwrap_or(false)
}

/// 版本号最大的匹配项（预发布按需排除）。
fn highest<'a>(
    names: impl Iterator<Item = (&'a String, &'a String)>,
    pattern: &str,
    prerelease: bool,
) -> Option<(String, String)> {
    names
        .filter(|(name, _)| glob_match(pattern, name))
        .filter_map(|(name, sha)| Version::parse(name).map(|v| (v, name, sha)))
        .filter(|(v, _, _)| prerelease || !v.is_prerelease())
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(a.1)))
        .map(|(_, name, sha)| (name.clone(), sha.clone()))
}

/// 按跟踪方式解析目标版本。`auto` 解析后的实际方式由 `effective` 给出。
pub(crate) fn resolve(track: &Track, refs: &RemoteRefs) -> Result<Target> {
    let default_branch = || -> Result<Target> {
        let name = refs
            .default_branch
            .clone()
            .ok_or_else(|| anyhow!("远端没有给出默认分支"))?;
        let commit = refs
            .heads
            .get(&name)
            .cloned()
            .ok_or_else(|| anyhow!("远端默认分支 {name} 不存在"))?;
        Ok(Target {
            kind: "branch",
            name,
            commit,
        })
    };
    match track {
        Track::Auto => match highest(refs.tags.iter(), DEFAULT_TAG_PATTERN, false) {
            Some((name, commit)) => Ok(Target {
                kind: "tag",
                name,
                commit,
            }),
            None => default_branch(),
        },
        Track::Tag {
            pattern,
            prerelease,
        } => highest(refs.tags.iter(), pattern, *prerelease)
            .map(|(name, commit)| Target {
                kind: "tag",
                name,
                commit,
            })
            .ok_or_else(|| anyhow!("没有匹配 {pattern} 的版本号标签")),
        Track::Default => default_branch(),
        Track::Branch(branch) => refs
            .heads
            .get(branch)
            .map(|commit| Target {
                kind: "branch",
                name: branch.clone(),
                commit: commit.clone(),
            })
            .ok_or_else(|| anyhow!("远端没有分支 {branch}")),
        Track::BranchPattern(pattern) => highest(refs.heads.iter(), pattern, true)
            .map(|(name, commit)| Target {
                kind: "branch",
                name,
                commit,
            })
            .ok_or_else(|| anyhow!("没有匹配 {pattern} 且带版本号的分支")),
        Track::Pinned(reference) => resolve_ref(reference, refs),
    }
}

/// 显式指定的版本：先按标签、再按分支、最后按提交（本地再核实提交存在）。
pub(crate) fn resolve_ref(reference: &str, refs: &RemoteRefs) -> Result<Target> {
    if let Some(commit) = refs.tags.get(reference) {
        return Ok(Target {
            kind: "tag",
            name: reference.to_string(),
            commit: commit.clone(),
        });
    }
    if let Some(commit) = refs.heads.get(reference) {
        return Ok(Target {
            kind: "branch",
            name: reference.to_string(),
            commit: commit.clone(),
        });
    }
    if is_sha(reference) {
        return Ok(Target {
            kind: "commit",
            name: reference.to_string(),
            commit: reference.to_string(),
        });
    }
    bail!("远端没有标签或分支 {reference}")
}

/// 界面用的版本清单：标签按版本号从新到旧（没有版本号的排最后），分支默认分支在前。
pub(crate) fn listing(refs: &RemoteRefs, limit: usize) -> Value {
    let mut tags: Vec<(Option<Version>, &String, &String)> = refs
        .tags
        .iter()
        .map(|(name, sha)| (Version::parse(name), name, sha))
        .collect();
    tags.sort_by(|a, b| match (&b.0, &a.0) {
        (Some(x), Some(y)) => x.cmp(y).then_with(|| a.1.cmp(b.1)),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => a.1.cmp(b.1),
    });
    let tags: Vec<Value> = tags
        .into_iter()
        .take(limit)
        .map(|(version, name, sha)| {
            json!({
                "name": name,
                "commit": sha,
                "version": version.is_some(),
                "prerelease": version.as_ref().is_some_and(Version::is_prerelease),
            })
        })
        .collect();
    let mut heads: Vec<(&String, &String)> = refs.heads.iter().collect();
    heads.sort_by_key(|(name, _)| {
        (
            Some(name.as_str()) != refs.default_branch.as_deref(),
            name.to_string(),
        )
    });
    let branches: Vec<Value> = heads
        .into_iter()
        .take(limit)
        .map(|(name, sha)| json!({"name": name, "commit": sha}))
        .collect();
    json!({
        "default_branch": refs.default_branch,
        "tags": tags,
        "tags_total": refs.tags.len(),
        "branches": branches,
        "branches_total": refs.heads.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SHA_C: &str = "cccccccccccccccccccccccccccccccccccccccc";
    const SHA_D: &str = "dddddddddddddddddddddddddddddddddddddddd";
    const SHA_E: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

    fn refs(lines: &[&str]) -> RemoteRefs {
        RemoteRefs::parse(&lines.join("\n"))
    }

    #[test]
    fn 版本号按数值比较_预发布低于正式版() {
        let v = |s: &str| Version::parse(s).unwrap();
        assert!(v("v2.10.0") > v("v2.9.0"));
        assert!(v("v1.4.99") > v("v1.4.98-rc.9"));
        assert!(v("v1.4.99") > v("v1.4.99-rc.10"));
        assert!(v("v1.4.99-rc.10") > v("v1.4.99-rc.9"));
        assert!(v("release/1.10") > v("release/1.9"));
        assert_eq!(v("v3.0"), v("3.0.0"));
        assert!(Version::parse("_gh-attach-assets").is_none());
        assert!(Version::parse("main").is_none());
    }

    #[test]
    fn 自动_有版本号标签跟最新正式版_注解标签取剥离后的提交() {
        let r = refs(&[
            &format!("ref: refs/heads/main\tHEAD"),
            &format!("{SHA_A}\tHEAD"),
            &format!("{SHA_A}\trefs/heads/main"),
            &format!("{SHA_B}\trefs/tags/v2.9.0"),
            &format!("{SHA_E}\trefs/tags/v2.10.0"),
            &format!("{SHA_C}\trefs/tags/v2.10.0^{{}}"),
            &format!("{SHA_D}\trefs/tags/v2.11.0-rc.1"),
            &format!("{SHA_D}\trefs/tags/_gh-attach-assets"),
        ]);
        let target = resolve(&Track::Auto, &r).unwrap();
        assert_eq!(
            target,
            Target {
                kind: "tag",
                name: "v2.10.0".into(),
                commit: SHA_C.into()
            }
        );
        assert_eq!(r.tag_at(SHA_C).as_deref(), Some("v2.10.0"));
        let with_pre = Track::parse(Some(&json!({"mode": "tag", "prerelease": true}))).unwrap();
        assert_eq!(resolve(&with_pre, &r).unwrap().name, "v2.11.0-rc.1");
    }

    #[test]
    fn 自动_没有版本号标签跟默认分支() {
        let r = refs(&[
            "ref: refs/heads/trunk\tHEAD",
            &format!("{SHA_A}\trefs/heads/trunk"),
            &format!("{SHA_B}\trefs/heads/codex/feature"),
            &format!("{SHA_C}\trefs/tags/_gh-attach-assets"),
        ]);
        assert_eq!(
            resolve(&Track::Auto, &r).unwrap(),
            Target {
                kind: "branch",
                name: "trunk".into(),
                commit: SHA_A.into()
            }
        );
    }

    #[test]
    fn 标签规则_指定分支_匹配分支与固定() {
        let r = refs(&[
            "ref: refs/heads/main\tHEAD",
            &format!("{SHA_A}\trefs/heads/main"),
            &format!("{SHA_B}\trefs/heads/release/1.9"),
            &format!("{SHA_C}\trefs/heads/release/1.10"),
            &format!("{SHA_D}\trefs/tags/v3.0.0"),
            &format!("{SHA_E}\trefs/tags/archify-dsh-v0.1.0"),
        ]);
        let tag = Track::parse(Some(&json!({"mode": "tag", "pattern": "v*"}))).unwrap();
        assert_eq!(resolve(&tag, &r).unwrap().name, "v3.0.0");
        let pattern = Track::parse(Some(
            &json!({"mode": "branch-pattern", "pattern": "release/*"}),
        ))
        .unwrap();
        assert_eq!(resolve(&pattern, &r).unwrap().name, "release/1.10");
        let branch =
            Track::parse(Some(&json!({"mode": "branch", "branch": "release/1.9"}))).unwrap();
        assert_eq!(resolve(&branch, &r).unwrap().commit, SHA_B);
        let pinned = Track::parse(Some(&json!({"mode": "pinned", "ref": "v3.0.0"}))).unwrap();
        assert_eq!(resolve(&pinned, &r).unwrap().commit, SHA_D);
        let commit = resolve_ref(&SHA_E[..10], &r).unwrap();
        assert_eq!(commit.kind, "commit");
        assert_eq!(commit.label(), "eeeeeee");
        assert!(resolve_ref("nope", &r).is_err());
        let missing = Track::parse(Some(&json!({"mode": "tag", "pattern": "x*"}))).unwrap();
        assert!(resolve(&missing, &r).is_err());
    }

    #[test]
    fn 跟踪方式解析拒绝坏值() {
        assert_eq!(Track::parse(None).unwrap(), Track::Auto);
        assert!(Track::parse(Some(&json!("tag"))).is_err());
        assert!(Track::parse(Some(&json!({"mode": "branch"}))).is_err());
        assert!(Track::parse(Some(
            &json!({"mode": "branch", "branch": "--upload-pack=x"})
        ))
        .is_err());
        assert!(Track::parse(Some(&json!({"mode": "pinned", "ref": "v*"}))).is_err());
        assert!(Track::parse(Some(&json!({"mode": "nightly"}))).is_err());
        let tag = Track::parse(Some(&json!({"mode": "tag"}))).unwrap();
        assert_eq!(
            tag.to_json(),
            json!({"mode": "tag", "pattern": "*", "prerelease": false})
        );
    }

    #[test]
    fn 版本清单_新版本在前_默认分支在前() {
        let r = refs(&[
            "ref: refs/heads/main\tHEAD",
            &format!("{SHA_A}\trefs/heads/dev"),
            &format!("{SHA_A}\trefs/heads/main"),
            &format!("{SHA_B}\trefs/tags/v2.9.0"),
            &format!("{SHA_C}\trefs/tags/v2.10.0"),
            &format!("{SHA_D}\trefs/tags/notes"),
        ]);
        let list = listing(&r, 50);
        let tags: Vec<&str> = list["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(tags, ["v2.10.0", "v2.9.0", "notes"]);
        assert_eq!(list["branches"][0]["name"], "main");
    }
}
