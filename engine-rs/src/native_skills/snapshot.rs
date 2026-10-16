use anyhow::{anyhow, bail, Context as _, Result};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use super::paths::{is_path_within, is_sha256_hex, resolve_loose, validate_symlink_internal};
use super::EXCLUDED_DIRS;

pub fn hash_skill_dir(dir: &Path) -> Result<String> {
    Ok(hash_entries(skill_entries(dir)?))
}

pub(super) fn skill_entries(dir: &Path) -> Result<Vec<(String, String)>> {
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    let mut buffer = [0u8; 64 * 1024];
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
                let mut file = std::fs::File::open(&child)
                    .with_context(|| format!("读取文件失败: {}", child.display()))?;
                let mut h = Sha256::new();
                loop {
                    let count = match file.read(&mut buffer) {
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        result => {
                            result.with_context(|| format!("读取文件失败: {}", child.display()))?
                        }
                    };
                    if count == 0 {
                        break;
                    }
                    h.update(&buffer[..count]);
                }
                entries.push((rel, format!("{:x}", h.finalize())));
            }
        }
    }
    Ok(entries)
}

pub(super) fn hash_entries(mut entries: Vec<(String, String)>) -> String {
    entries.sort();
    let mut h = Sha256::new();
    for (rel, digest) in &entries {
        h.update(rel.as_bytes());
        h.update(b"\0");
        h.update(digest.as_bytes());
        h.update(b"\0");
    }
    format!("{:x}", h.finalize())
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

pub fn snapshot_skill(source_dir: &Path, data_dir: &Path, version: &str) -> Result<PathBuf> {
    if !is_sha256_hex(version) {
        bail!("Invalid skill snapshot version.");
    }
    // The caller binds this exact version in its assignment transaction.
    let version_dir = data_dir.join("skill-versions").join(version);
    let content_path = version_dir.join("content");
    match std::fs::symlink_metadata(&version_dir) {
        Ok(_) => return verify_skill_version(data_dir, version),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let parent = data_dir.join("skill-versions");
    std::fs::create_dir_all(&parent)?;
    let staging = super::snapshot_storage::Staging::create(&parent, version)?;
    let copied = (|| -> Result<()> {
        let content = staging.path().join("content");
        copy_dir_recursive(source_dir, &content)?;
        let actual = hash_skill_dir(&content)?;
        if actual != version {
            bail!("Skill snapshot copy does not match the chosen version {version} (actual {actual}); refusing to publish.");
        }
        staging.publish(&version_dir)?;
        Ok(())
    })();
    if let Err(error) = copied {
        let cleanup = staging.cleanup();
        if let Err(cleanup) = cleanup {
            return Err(error.context(format!("Snapshot staging cleanup failed: {cleanup}")));
        }
        return Err(error);
    }
    Ok(content_path)
}

pub fn verify_skill_version(data_dir: &Path, version: &str) -> Result<PathBuf> {
    if !is_sha256_hex(version) {
        bail!("version 格式非法: {:?}", version);
    }
    let parent = data_dir.join("skill-versions");
    let version_dir = parent.join(version);
    let content_path = version_dir.join("content");
    let (_parent_file, parent_id) = super::snapshot_storage::open_directory(&parent)?;
    let (_version_file, version_id) = super::snapshot_storage::open_directory(&version_dir)?;
    let (_content_file, content_id) = super::snapshot_storage::open_directory(&content_path)?;
    let actual = hash_skill_dir(&content_path)?;
    if actual != version {
        bail!(
            "版本快照 {} 内容不一致（实际 {}），拒绝使用",
            version,
            actual
        );
    }
    super::snapshot_storage::unchanged(&parent, parent_id)?;
    super::snapshot_storage::unchanged(&version_dir, version_id)?;
    super::snapshot_storage::unchanged(&content_path, content_id)?;
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
    let mut conflicts = BTreeSet::new();
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
            if let Some(previous) = found.get(&name) {
                if previous == search_path {
                    continue;
                }
                duplicates.insert(name.clone());
                conflicts.insert(format!(
                    "{}: {} / {}",
                    name,
                    previous.display(),
                    search_path.display()
                ));
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
                if let Some(previous) = found.get(&name) {
                    if previous == &path {
                        continue;
                    }
                    duplicates.insert(name.clone());
                    conflicts.insert(format!(
                        "{}: {} / {}",
                        name,
                        previous.display(),
                        path.display()
                    ));
                } else {
                    found.insert(name, path);
                }
            }
        }
    }
    if !duplicates.is_empty() {
        return Err(SkillPathsError::new(
            "duplicate-name",
            Some(duplicates.into_iter().collect::<Vec<_>>().join(", ")),
            format!(
                "同一来源包含重名技能，请调整目录或 skill_paths 后再部署：{}",
                conflicts.into_iter().collect::<Vec<_>>().join("; ")
            ),
        )
        .into());
    }
    Ok((found, duplicates))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_existing_version_is_preserved_instead_of_republished() {
        let root = std::env::temp_dir().join(format!(
            "am-incomplete-snapshot-{}",
            crate::store::uuid_v4()
        ));
        let source = root.join("source");
        let data = root.join("data");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("SKILL.md"), b"selected content").unwrap();
        let version = hash_skill_dir(&source).unwrap();
        let existing = data.join("skill-versions").join(&version);
        std::fs::create_dir_all(&existing).unwrap();
        let result = snapshot_skill(&source, &data, &version);
        assert!(
            result.is_err(),
            "An existing incomplete version was replaced"
        );
        assert!(existing.is_dir());
        assert_eq!(std::fs::read_dir(&existing).unwrap().count(), 0);
        assert_eq!(
            std::fs::read_dir(data.join("skill-versions"))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(
            std::fs::read(source.join("SKILL.md")).unwrap(),
            b"selected content"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn version_and_content_directory_links_are_refused_without_touching_targets() {
        let root = std::env::temp_dir().join(format!(
            "am-snapshot-root-links-{}",
            crate::store::uuid_v4()
        ));
        let source = root.join("source");
        let data = root.join("data");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("SKILL.md"), b"selected content").unwrap();
        let version = hash_skill_dir(&source).unwrap();
        let content = snapshot_skill(&source, &data, &version).unwrap();
        let version_dir = content.parent().unwrap();
        let moved_version = root.join("moved-version");
        std::fs::rename(version_dir, &moved_version).unwrap();
        crate::platform::link_dir(&moved_version, version_dir).unwrap();
        assert!(verify_skill_version(&data, &version).is_err());
        assert!(snapshot_skill(&source, &data, &version).is_err());
        assert_eq!(
            std::fs::read(moved_version.join("content/SKILL.md")).unwrap(),
            b"selected content"
        );
        crate::platform::remove_link(version_dir).unwrap();
        std::fs::rename(&moved_version, version_dir).unwrap();
        let moved_content = root.join("moved-content");
        std::fs::rename(&content, &moved_content).unwrap();
        crate::platform::link_dir(&moved_content, &content).unwrap();
        assert!(verify_skill_version(&data, &version).is_err());
        assert!(snapshot_skill(&source, &data, &version).is_err());
        assert_eq!(
            std::fs::read(moved_content.join("SKILL.md")).unwrap(),
            b"selected content"
        );
        crate::platform::remove_link(&content).unwrap();
        std::fs::rename(moved_content, &content).unwrap();
        assert_eq!(verify_skill_version(&data, &version).unwrap(), content);
        #[cfg(windows)]
        super::super::fork::make_writable(&root).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_content_cannot_publish_a_snapshot_or_replace_an_existing_version() {
        let root =
            std::env::temp_dir().join(format!("am-snapshot-verify-{}", crate::store::uuid_v4()));
        let source = root.join("source");
        let data = root.join("data");
        std::fs::create_dir_all(&source).unwrap();
        let file = source.join("SKILL.md");
        std::fs::write(&file, b"original version").unwrap();
        let original = hash_skill_dir(&source).unwrap();
        let original_path = snapshot_skill(&source, &data, &original).unwrap();
        assert_eq!(
            verify_skill_version(&data, &original).unwrap(),
            original_path
        );
        let other_staging = data.join("skill-versions/.staging-unrelated");
        std::fs::create_dir(&other_staging).unwrap();
        std::fs::write(other_staging.join("marker"), b"keep unrelated staging").unwrap();

        std::fs::write(&file, b"chosen version").unwrap();
        let chosen = hash_skill_dir(&source).unwrap();
        // An external source change occurs after the assignment chooses its hash.
        std::fs::write(&file, b"external version").unwrap();
        #[cfg(windows)]
        {
            let mut permissions = std::fs::metadata(&file).unwrap().permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(&file, permissions).unwrap();
        }
        let error = snapshot_skill(&source, &data, &chosen).unwrap_err();
        assert!(error
            .to_string()
            .contains("does not match the chosen version"));
        assert!(!data.join("skill-versions").join(&chosen).exists());
        let versions: BTreeSet<_> = std::fs::read_dir(data.join("skill-versions"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            versions,
            [original.clone(), ".staging-unrelated".into()]
                .into_iter()
                .collect(),
            "Failed staging must be removed"
        );
        assert_eq!(
            std::fs::read(other_staging.join("marker")).unwrap(),
            b"keep unrelated staging"
        );
        assert_eq!(
            std::fs::read(original_path.join("SKILL.md")).unwrap(),
            b"original version"
        );
        assert_eq!(std::fs::read(&file).unwrap(), b"external version");
        #[cfg(windows)]
        assert!(std::fs::metadata(&file).unwrap().permissions().readonly());
        let current = hash_skill_dir(&source).unwrap();
        let current_path = snapshot_skill(&source, &data, &current).unwrap();
        assert_eq!(verify_skill_version(&data, &current).unwrap(), current_path);
        assert_eq!(
            std::fs::read(current_path.join("SKILL.md")).unwrap(),
            b"external version"
        );
        assert_eq!(
            verify_skill_version(&data, &original).unwrap(),
            original_path
        );
        #[cfg(windows)]
        super::super::fork::make_writable(&root).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn chunked_hash_preserves_existing_snapshot_digest() {
        let root = std::env::temp_dir().join(format!(
            "am-hash-chunks-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("SKILL.md"), "# Hash fixture\n").unwrap();
        std::fs::write(root.join("empty"), []).unwrap();
        std::fs::write(root.join(".git/ignored"), "excluded content").unwrap();
        let mut payload: Vec<u8> = (0..131_089).map(|index| (index % 251) as u8).collect();
        std::fs::write(root.join("assets/payload.bin"), &payload).unwrap();
        // Golden digest from the existing whole-file format, spanning two full
        // buffers, a partial buffer, an empty file, and excluded Git metadata.
        let expected = "9c0097a54c817ee20234af24936f2ffec9dc2e2d91a70a6d6fb65f9eeab99444";
        assert_eq!(hash_skill_dir(&root).unwrap(), expected);
        payload[65_536] ^= 1;
        std::fs::write(root.join("assets/payload.bin"), payload).unwrap();
        assert_ne!(hash_skill_dir(&root).unwrap(), expected);
        std::fs::remove_dir_all(root).unwrap();
    }

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
