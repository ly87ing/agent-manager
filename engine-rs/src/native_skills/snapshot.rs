use anyhow::{anyhow, bail, Context as _, Result};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::paths::{is_path_within, is_sha256_hex, resolve_loose, validate_symlink_internal};
use super::EXCLUDED_DIRS;

pub fn hash_skill_dir(dir: &Path) -> Result<String> {
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let read = std::fs::read_dir(&current)
            .with_context(|| format!("读取目录失败: {}", current.display()))?;
        let mut children = Vec::new();
        for entry in read {
            let entry = entry?;
            children.push(entry.path());
        }
        children.sort();
        for child in children {
            let meta = std::fs::symlink_metadata(&child)?;
            let rel = child
                .strip_prefix(dir)
                .map_err(|_| anyhow!("快照路径越界: {}", child.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            if meta.is_dir() {
                let name = child
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if !EXCLUDED_DIRS.contains(&name.as_str()) {
                    stack.push(child);
                }
            } else if meta.file_type().is_symlink() {
                let target = crate::platform::read_link(&child)?;
                validate_symlink_internal(dir, &child, &target)?;
                let mut h = Sha256::new();
                h.update(b"symlink:");
                h.update(target.to_string_lossy().as_bytes());
                entries.push((rel, format!("{:x}", h.finalize())));
            } else if meta.is_file() {
                let bytes = std::fs::read(&child)
                    .with_context(|| format!("读取文件失败: {}", child.display()))?;
                let mut h = Sha256::new();
                h.update(&bytes);
                entries.push((rel, format!("{:x}", h.finalize())));
            }
        }
    }
    entries.sort();
    let mut h = Sha256::new();
    for (rel, digest) in &entries {
        h.update(rel.as_bytes());
        h.update(b"\0");
        h.update(digest.as_bytes());
        h.update(b"\0");
    }
    Ok(format!("{:x}", h.finalize()))
}

pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    let mut stack = vec![src.to_path_buf()];
    while let Some(current) = stack.pop() {
        let read = std::fs::read_dir(&current)?;
        let mut children: Vec<PathBuf> = Vec::new();
        for entry in read {
            children.push(entry?.path());
        }
        children.sort();
        for child in children {
            let meta = std::fs::symlink_metadata(&child)?;
            let rel = child
                .strip_prefix(src)
                .map_err(|_| anyhow!("快照路径越界: {}", child.display()))?;
            let out = dst.join(rel);
            if meta.is_dir() {
                let name = child
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if EXCLUDED_DIRS.contains(&name.as_str()) {
                    continue;
                }
                std::fs::create_dir_all(&out)?;
                stack.push(child);
            } else if meta.file_type().is_symlink() {
                let target = crate::platform::read_link(&child)?;
                validate_symlink_internal(src, &child, &target)?;
                // 相对目标按链接所在目录解析，判断指向的是目录还是文件（Windows 两种链接不同）
                let resolved = child.parent().unwrap_or(src).join(&target);
                if resolved.is_dir() {
                    crate::platform::link_dir(&target, &out)?;
                } else {
                    crate::platform::link_file(&target, &out)?;
                }
            } else if meta.is_file() {
                std::fs::copy(&child, &out)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o444));
                }
            }
        }
    }
    Ok(())
}

pub fn snapshot_skill(source_dir: &Path, data_dir: &Path) -> Result<PathBuf> {
    let version = hash_skill_dir(source_dir)?;
    let version_dir = data_dir.join("skill-versions").join(&version);
    let content_path = version_dir.join("content");
    if content_path.is_dir() {
        let existing = hash_skill_dir(&content_path)?;
        if existing == version {
            return Ok(content_path);
        }
        bail!(
            "版本快照 {} 内容被篡改（期望 {} 实际 {}），拒绝复用",
            version_dir.display(),
            version,
            existing
        );
    }
    let staging = data_dir
        .join("skill-versions")
        .join(format!(".staging-{}", version));
    if staging.exists() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    copy_dir_recursive(source_dir, &staging.join("content"))?;
    std::fs::create_dir_all(data_dir.join("skill-versions"))?;
    std::fs::rename(&staging, &version_dir)?;
    Ok(content_path)
}

pub fn verify_skill_version(data_dir: &Path, version: &str) -> Result<PathBuf> {
    if !is_sha256_hex(version) {
        bail!("version 格式非法: {:?}", version);
    }
    let content_path = data_dir
        .join("skill-versions")
        .join(version)
        .join("content");
    if !content_path.is_dir() {
        bail!("版本快照不存在: {}", content_path.display());
    }
    let actual = hash_skill_dir(&content_path)?;
    if actual != version {
        bail!(
            "版本快照 {} 内容不一致（实际 {}），拒绝使用",
            version,
            actual
        );
    }
    Ok(content_path)
}

/// skill_paths 解析失败的结构化信息：status 用于呈现 discovery_error
/// （category 为固定词汇表，item 为出问题的 skill_paths 项），apply 路径
/// 仍以 anyhow 错误消息上抛，消息文本与历史一致、不含敏感正文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillPathsError {
    pub category: &'static str,
    pub item: Option<String>,
    pub detail: String,
}

impl SkillPathsError {
    fn new(category: &'static str, item: Option<String>, detail: String) -> anyhow::Error {
        anyhow::Error::new(Self {
            category,
            item,
            detail,
        })
    }
}

impl std::fmt::Display for SkillPathsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for SkillPathsError {}

pub fn resolve_skill_search_paths(
    repo_dir: &Path,
    def: &Map<String, Value>,
) -> Result<Vec<PathBuf>> {
    let repo_canonical = resolve_loose(repo_dir);
    match def.get("skill_paths") {
        None | Some(Value::Null) => Ok(vec![repo_canonical]),
        Some(Value::Array(items)) => {
            if items.is_empty() {
                return Err(SkillPathsError::new(
                    "empty-list",
                    None,
                    "skill_paths 必须是非空数组".to_string(),
                ));
            }
            let mut paths = Vec::new();
            for item in items {
                let rel = item.as_str().ok_or_else(|| {
                    SkillPathsError::new(
                        "non-string-item",
                        None,
                        "skill_paths 项必须是字符串".to_string(),
                    )
                })?;
                if rel.is_empty() || rel.contains('\0') {
                    return Err(SkillPathsError::new(
                        "invalid-path",
                        Some(rel.to_string()),
                        "skill_paths 包含非法路径".to_string(),
                    ));
                }
                let mut resolved = resolve_loose(&repo_canonical.join(rel));
                // 来源把顶层技能目录收进了 skills/：旧定义里的单段技能名在顶层找不到时，
                // 按 skills/<名> 找，旧定义不用改也能继续发现。
                if !resolved.is_dir() && is_single_segment(rel) {
                    let moved = resolve_loose(&repo_canonical.join("skills").join(rel));
                    if moved.is_dir() {
                        resolved = moved;
                    }
                }
                if !is_path_within(&resolved, &repo_canonical) {
                    return Err(SkillPathsError::new(
                        "out-of-bounds",
                        Some(rel.to_string()),
                        format!("skill_paths 包含越界路径: {}", rel),
                    ));
                }
                if !resolved.is_dir() {
                    return Err(SkillPathsError::new(
                        "missing-dir",
                        Some(rel.to_string()),
                        format!("skill_paths 指向的目录不存在: {}", rel),
                    ));
                }
                paths.push(resolved);
            }
            Ok(paths)
        }
        _ => Err(SkillPathsError::new(
            "invalid-shape",
            None,
            "skill_paths 必须是字符串数组或 null".to_string(),
        )),
    }
}

fn is_single_segment(rel: &str) -> bool {
    let mut parts = Path::new(rel).components();
    matches!(
        (parts.next(), parts.next()),
        (Some(std::path::Component::Normal(_)), None)
    )
}

/// 来源里全部可用的技能（不受 skill_paths 限制），给「用哪些技能」列候选：
/// (技能名, 相对来源根的路径)。含 SKILL.md 的目录即一个技能，不再往里找；最多往下三层，
/// 跳过隐藏目录与 EXCLUDED_DIRS。来源根本身就是技能时路径为空串。同名只留先找到的。只读。
pub fn available_repo_skills(repo_dir: &Path) -> Vec<(String, String)> {
    fn walk(dir: &Path, rel: &str, depth: usize, out: &mut Vec<(String, String)>) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = read.flatten().map(|entry| entry.path()).collect();
        entries.sort();
        for path in entries {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if !path.is_dir() || name.starts_with('.') || EXCLUDED_DIRS.contains(&name.as_str()) {
                continue;
            }
            let child = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if path.join("SKILL.md").is_file() {
                if !out.iter().any(|(existing, _)| existing == &name) {
                    out.push((name, child));
                }
            } else if depth < 3 {
                walk(&path, &child, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    if repo_dir.join("SKILL.md").is_file() {
        let name = repo_dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        out.push((name, String::new()));
        return out;
    }
    walk(repo_dir, "", 1, &mut out);
    out
}

pub fn discover_repo_skills(
    repo_dir: &Path,
    def: &Map<String, Value>,
) -> Result<(BTreeMap<String, PathBuf>, BTreeSet<String>)> {
    let mut found = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    if !repo_dir.is_dir() {
        return Ok((found, duplicates));
    }
    let search_paths = resolve_skill_search_paths(repo_dir, def)?;
    for search_path in &search_paths {
        if search_path.join("SKILL.md").is_file() {
            let name = search_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if found.contains_key(&name) {
                duplicates.insert(name.clone());
            } else {
                found.insert(name, search_path.clone());
            }
            continue;
        }
        let read = match std::fs::read_dir(search_path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let mut entries = Vec::new();
        for entry in read {
            if let Ok(entry) = entry {
                entries.push(entry.path());
            }
        }
        entries.sort();
        for path in entries {
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if EXCLUDED_DIRS.contains(&name.as_str()) {
                continue;
            }
            let is_skill = path.join("SKILL.md").is_file() || def.get("skill_paths").is_some();
            if is_skill {
                if found.contains_key(&name) {
                    duplicates.insert(name.clone());
                } else {
                    found.insert(name, path);
                }
            }
        }
    }
    Ok((found, duplicates))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// 读不了的内容是「未知」，不能给出可比较的指纹：两个读不了的目录若被当成相同，
    /// 去重或撤回就可能删掉其中一份（Magpie 9fb9a8ab 的教训）。
    #[cfg(unix)]
    #[test]
    fn unreadable_content_is_an_error_not_a_comparable_fingerprint() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("am-hash-unreadable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (a, b) = (root.join("a"), root.join("b"));
        for (dir, body) in [(&a, "one"), (&b, "two")] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("SKILL.md"), body).unwrap();
            std::fs::set_permissions(dir.join("SKILL.md"), std::fs::Permissions::from_mode(0o000))
                .unwrap();
        }
        // root 能无视权限读到内容，这时检查不了
        if std::fs::read(a.join("SKILL.md")).is_ok() {
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        assert!(hash_skill_dir(&a).is_err());
        assert!(hash_skill_dir(&b).is_err());
        // 读不了的子目录同样报错
        std::fs::set_permissions(a.join("SKILL.md"), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        let sub = a.join("scripts");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert!(hash_skill_dir(&a).is_err());
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(b.join("SKILL.md"), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }
}
