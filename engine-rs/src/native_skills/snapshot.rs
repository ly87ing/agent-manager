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
                let target = std::fs::read_link(&child)?;
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

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
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
                let target = std::fs::read_link(&child)?;
                validate_symlink_internal(src, &child, &target)?;
                #[cfg(unix)]
                std::os::unix::fs::symlink(&target, &out)?;
                #[cfg(windows)]
                std::os::windows::fs::symlink_file(&target, &out)?;
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
                let path = repo_canonical.join(rel);
                let resolved = resolve_loose(&path);
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
