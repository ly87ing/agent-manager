use anyhow::{anyhow, bail, Context as _, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{CollectionDef, ParsedTarget};
use crate::native::Context;
use crate::store;

pub fn validate_path_identifier(name: &str, kind: &str) -> Result<()> {
    if name.is_empty() {
        bail!("{} 不能为空", kind);
    }
    if name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        bail!("{} 包含非法路径字符或逃逸命名: {:?}", kind, name);
    }
    Ok(())
}

pub fn parse_only(only: &str) -> Result<ParsedTarget> {
    let norm = only.strip_prefix("skill:").unwrap_or(only);
    if norm.is_empty() {
        bail!("非法 skill 目标: {:?}", only);
    }
    let (coll, skill) = match norm.split_once('/') {
        Some((c, s)) => (c.to_string(), Some(s.to_string())),
        None => (norm.to_string(), None),
    };
    validate_path_identifier(&coll, "集合名称")?;
    if let Some(ref s) = skill {
        validate_path_identifier(s, "Skill 名称")?;
    }
    Ok(ParsedTarget {
        collection: coll,
        skill,
    })
}

pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub fn expand_home(path: &str, home_dir: &Path) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        return home_dir.join(rest);
    }
    if path == "~" {
        return home_dir.to_path_buf();
    }
    PathBuf::from(path)
}

pub fn resolve_loose(path: &Path) -> PathBuf {
    if let Ok(canon) = path.canonicalize() {
        return canon;
    }
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::Prefix(p) => out.push(p.as_os_str()),
            std::path::Component::RootDir => out.push("/"),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::Normal(c) => out.push(c),
        }
    }
    out
}

pub fn is_path_within(path: &Path, base: &Path) -> bool {
    let path = resolve_loose(path);
    let base = resolve_loose(base);
    path == base || path.starts_with(&base)
}

pub fn repo_base_dir(ctx: &Context) -> PathBuf {
    ctx.data_dir.join("repositories")
}

/// manager 自管基目录（git 集合缓存与内容快照），与集合定义无关。
pub fn manager_bases(ctx: &Context) -> Vec<PathBuf> {
    vec![repo_base_dir(ctx), ctx.data_dir.join("skill-versions")]
}

/// 受管基准：自管基目录 + 已登记的本地来源集合源根。
/// 本地来源集合的链接直接指向外部源目录，源根进入基准后这些链接才被视为「manager 可管」
/// （可被覆盖、停用时删除）；未登记的外部路径仍是外部冲突，fail-closed 不覆盖。
pub fn managed_bases(ctx: &Context, collections: &BTreeMap<String, CollectionDef>) -> Vec<PathBuf> {
    let mut bases = manager_bases(ctx);
    for coll in collections.values() {
        if coll.deleted || coll.source_path.is_none() {
            continue;
        }
        bases.push(collection_source_root(ctx, coll));
    }
    bases
}

/// 集合是否为本地来源（直链模式）。
pub fn is_local_source(coll: &CollectionDef) -> bool {
    coll.source_path.is_some()
}

/// 集合源根：本地来源集合为登记的 source_path（展开 ~ 后按词法归一），
/// git 集合为 manager 自管的仓库缓存目录。
pub fn collection_source_root(ctx: &Context, coll: &CollectionDef) -> PathBuf {
    match coll.source_path.as_deref() {
        Some(raw) => resolve_loose(&expand_home(raw, &ctx.home_dir)),
        None => repo_base_dir(ctx).join(&coll.name),
    }
}

/// 期望链接目标（只读路径计算，不做存在性/快照校验，供 status 判定漂移）：
/// 本地来源集合指向源目录下的同名 skill，git 集合指向内容快照目录。
pub fn expected_link_path(
    ctx: &Context,
    coll: &CollectionDef,
    skill: &str,
    version: Option<&str>,
) -> PathBuf {
    if is_local_source(coll) {
        collection_source_root(ctx, coll).join(skill)
    } else {
        ctx.data_dir
            .join("skill-versions")
            .join(version.unwrap_or(""))
            .join("content")
    }
}

/// 写路径的期望链接目标（带校验，fail-closed）：本地来源集合校验源目录仍在且不越界，
/// git 集合走既有内容快照校验（哈希必须与 version 一致）。
pub fn expected_link_target(
    ctx: &Context,
    coll: &CollectionDef,
    skill: &str,
    version: &str,
) -> Result<PathBuf> {
    if !is_local_source(coll) {
        return super::snapshot::verify_skill_version(&ctx.data_dir, version);
    }
    let root = collection_source_root(ctx, coll);
    let target = resolve_loose(&root.join(skill));
    if !is_path_within(&target, &root) {
        bail!("[{}/{}] 源路径越界，拒绝链接", coll.name, skill);
    }
    if !target.is_dir() {
        bail!(
            "[{}/{}] 本地来源中找不到该技能目录（源已被移动或删除）",
            coll.name,
            skill
        );
    }
    Ok(target)
}

/// 本地来源集合的源目录安全校验（fail-closed，写路径与只读 status 同一口径）：
/// 必须是已存在的真实目录（不接受符号链接根，避免源被改指后静默换内容）；
/// 不得落在 manager 自管数据目录内，也不得是任一 Agent 技能目录本身或其祖先/子孙
/// ——否则链接会自指或让 manager 覆盖助手目录。
pub fn validate_local_source(
    ctx: &Context,
    coll: &CollectionDef,
    agent_targets: &BTreeMap<String, PathBuf>,
) -> Result<PathBuf> {
    let Some(raw) = coll.source_path.as_deref() else {
        bail!("[{}] 不是本地来源集合", coll.name);
    };
    if raw.trim().is_empty() {
        bail!("[{}] source_path 不能为空", coll.name);
    }
    if coll.url.is_some() {
        bail!(
            "[{}] source_path 与 url 互斥：本地来源集合不从 git 同步",
            coll.name
        );
    }
    let root = collection_source_root(ctx, coll);
    if is_link_entry(&root) {
        bail!("[{}] source_path 不能是符号链接", coll.name);
    }
    if !root.is_dir() {
        bail!("[{}] source_path 指向的目录不存在", coll.name);
    }
    let data_dir = resolve_loose(&ctx.data_dir);
    if is_path_within(&root, &data_dir) {
        bail!(
            "[{}] source_path 不得位于 Agent Manager 数据目录内",
            coll.name
        );
    }
    for (agent, skills_dir) in agent_targets {
        let target = resolve_loose(skills_dir);
        if root == target || is_path_within(&root, &target) || is_path_within(&target, &root) {
            bail!(
                "[{}] source_path 与 Agent '{}' 的技能目录重叠，拒绝自指",
                coll.name,
                agent
            );
        }
    }
    Ok(root)
}

// ---------------------------------------------------------------------------
// 符号链接安全操作
// ---------------------------------------------------------------------------

pub fn is_link_entry(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

pub fn resolve_link_target(link_path: &Path) -> Result<PathBuf> {
    let raw = std::fs::read_link(link_path)
        .with_context(|| format!("读取符号链接失败: {}", link_path.display()))?;
    let resolved = if raw.is_absolute() {
        raw
    } else {
        link_path.parent().map(|p| p.join(&raw)).unwrap_or(raw)
    };
    Ok(resolve_loose(&resolved))
}

pub fn is_managed_link(link_path: &Path, bases: &[PathBuf]) -> bool {
    if !is_link_entry(link_path) {
        return false;
    }
    match resolve_link_target(link_path) {
        Ok(target) => bases.iter().any(|base| is_path_within(&target, base)),
        Err(_) => false,
    }
}

pub fn check_not_external_conflict(link_path: &Path, bases: &[PathBuf]) -> Result<()> {
    check_not_external_conflict_with_takeover(link_path, bases, false)
}

/// 入口为指向受管基目录之外的符号链接（旧体系遗留的计划外链接）。
/// 真实文件/目录不属于可接管对象，仍由 fail-closed 检查拦截。
pub fn is_unmanaged_link(link_path: &Path, bases: &[PathBuf]) -> bool {
    is_link_entry(link_path) && !is_managed_link(link_path, bases)
}

/// 外部冲突检查（含显式接管通道）：入口不存在或链接指向受管基目录内 → 放行；
/// 非受管符号链接仅在 allow_takeover=true 时放行（逐 skill add/remove 的显式接管）；
/// 真实文件/目录与缺旗标的非受管链接一律 fail-closed。
pub fn check_not_external_conflict_with_takeover(
    link_path: &Path,
    bases: &[PathBuf],
    allow_takeover: bool,
) -> Result<()> {
    if !link_path.exists() && !is_link_entry(link_path) {
        return Ok(());
    }
    if is_managed_link(link_path, bases) {
        return Ok(());
    }
    if is_link_entry(link_path) {
        if allow_takeover {
            return Ok(());
        }
        bail!(
            "目标路径已存在外部非受管链接 ({})，拒绝静默覆盖或删除以防损坏用户文件；如确认接管或移除该链接，请显式携带 --allow-takeover",
            link_path.display()
        );
    }
    bail!(
        "目标路径已存在外部非受管内容 ({})，拒绝静默覆盖或删除以防损坏用户文件",
        link_path.display()
    );
}

pub fn check_shared_agent_target(
    agent: &str,
    agent_targets: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    let target = match agent_targets.get(agent) {
        Some(t) => resolve_loose(t),
        None => return Ok(()),
    };
    for (other, other_target) in agent_targets {
        if other == agent {
            continue;
        }
        if resolve_loose(other_target) == target {
            bail!(
                "Agent '{}' 的技能目标目录与 Agent '{}' 共享 ({})，单 Agent 操作明确拒绝以防破坏其他 Agent 的隔离状态",
                agent, other, target.display()
            );
        }
    }
    Ok(())
}

pub fn atomic_create_or_replace_link(source: &Path, link_path: &Path) -> Result<()> {
    let source = resolve_loose(source);
    let parent = link_path
        .parent()
        .ok_or_else(|| anyhow!("链接路径无父目录: {}", link_path.display()))?;
    std::fs::create_dir_all(parent)?;
    if link_path.exists() && !is_link_entry(link_path) {
        bail!(
            "目标入口已存在外部非受管内容，拒绝覆盖: {}",
            link_path.display()
        );
    }
    let file_name = link_path
        .file_name()
        .ok_or_else(|| anyhow!("链接路径无文件名: {}", link_path.display()))?
        .to_string_lossy();
    let tmp = parent.join(format!(".{}.tmp-{}", file_name, store::uuid_v4()));
    #[cfg(unix)]
    std::os::unix::fs::symlink(&source, &tmp)
        .with_context(|| format!("创建临时符号链接失败: {}", tmp.display()))?;
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&source, &tmp)
        .with_context(|| format!("创建临时符号链接失败: {}", tmp.display()))?;

    if let Err(err) = std::fs::rename(&tmp, link_path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err).with_context(|| format!("原子替换链接失败: {}", link_path.display()));
    }
    Ok(())
}

pub fn safe_remove_link(link_path: &Path, bases: &[PathBuf]) -> Result<()> {
    safe_remove_link_with_takeover(link_path, bases, false)
}

/// 含显式接管通道的移除：allow_takeover=true 时允许删除非受管符号链接；
/// 真实文件/目录仍 fail-closed，绝不删除。
pub fn safe_remove_link_with_takeover(
    link_path: &Path,
    bases: &[PathBuf],
    allow_takeover: bool,
) -> Result<()> {
    if !link_path.exists() && !is_link_entry(link_path) {
        return Ok(());
    }
    check_not_external_conflict_with_takeover(link_path, bases, allow_takeover)?;
    if is_link_entry(link_path) {
        std::fs::remove_file(link_path)
            .with_context(|| format!("删除符号链接失败: {}", link_path.display()))?;
    }
    Ok(())
}

pub fn restore_link_guarded(
    link_path: &Path,
    old_target: Option<PathBuf>,
    expected: &ExpectedLinkState,
) -> Result<()> {
    match expected {
        ExpectedLinkState::Present(exp_target) => {
            if !is_link_entry(link_path) {
                bail!(
                    "回滚跳过: 路径 '{}' 已被外部修改（期望为软链接，实际为非链接文件/目录或已缺失），保留外部改动绝不覆盖",
                    link_path.display()
                );
            }
            let current_target = resolve_link_target(link_path)?;
            if resolve_loose(&current_target) != resolve_loose(exp_target) {
                bail!(
                    "回滚跳过: 路径 '{}' 已被外部重定向（当前指向 {}, 期望指向 {}），保留外部改动绝不覆盖",
                    link_path.display(),
                    current_target.display(),
                    exp_target.display()
                );
            }
            match old_target {
                Some(old) => {
                    atomic_create_or_replace_link(&old, link_path)?;
                }
                None => {
                    if is_link_entry(link_path) {
                        std::fs::remove_file(link_path).with_context(|| {
                            format!("回滚删除软链接失败: {}", link_path.display())
                        })?;
                    }
                }
            }
        }
        ExpectedLinkState::Absent => {
            let exists = link_path.exists() || is_link_entry(link_path);
            if exists {
                bail!(
                    "回滚跳过: 路径 '{}' 已被外部创建，保留外部改动绝不覆盖",
                    link_path.display()
                );
            }
            if let Some(old) = old_target {
                atomic_create_or_replace_link(&old, link_path)?;
            }
        }
    }
    Ok(())
}

pub fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                components.pop();
            }
            c => components.push(c),
        }
    }
    components.iter().collect()
}

pub fn validate_symlink_internal(
    skill_root: &Path,
    symlink_path: &Path,
    target: &Path,
) -> Result<()> {
    if target.is_absolute() {
        bail!(
            "Skill 快照内软链接禁止使用绝对路径: {} -> {}",
            symlink_path.display(),
            target.display()
        );
    }
    let parent = symlink_path.parent().unwrap_or(skill_root);
    let resolved = parent.join(target);
    let norm_root = normalize_lexical_path(skill_root);
    let norm_target = normalize_lexical_path(&resolved);
    if !norm_target.starts_with(&norm_root) {
        bail!(
            "Skill 快照内软链接逃逸出技能根目录: {} -> {}",
            symlink_path.display(),
            target.display()
        );
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpectedLinkState {
    Present(PathBuf),
    Absent,
}

#[derive(Debug, Clone)]
pub struct LinkMutation {
    pub link_path: PathBuf,
    pub old_target: Option<PathBuf>,
    pub expected: ExpectedLinkState,
}

pub struct RollbackGuard {
    pub mutations: Vec<LinkMutation>,
    pub armed: bool,
}

impl RollbackGuard {
    pub fn new() -> Self {
        Self {
            mutations: Vec::new(),
            armed: true,
        }
    }

    pub fn record_mutation(
        &mut self,
        link_path: &Path,
        old_target: Option<PathBuf>,
        expected: ExpectedLinkState,
    ) {
        self.mutations.push(LinkMutation {
            link_path: link_path.to_path_buf(),
            old_target,
            expected,
        });
    }

    pub fn rollback(&mut self) -> Result<()> {
        if !self.armed {
            return Ok(());
        }
        let mut conflicts = Vec::new();
        while let Some(m) = self.mutations.pop() {
            if let Err(err) = restore_link_guarded(&m.link_path, m.old_target, &m.expected) {
                conflicts.push(err.to_string());
            }
        }
        if !conflicts.is_empty() {
            bail!(
                "回滚检测到外部修改冲突或失败，为保护外部改动已拒绝覆盖: {}",
                conflicts.join("; ")
            );
        }
        Ok(())
    }

    pub fn disarm(&mut self) {
        self.armed = false;
        self.mutations.clear();
    }
}

impl Drop for RollbackGuard {
    fn drop(&mut self) {
        let _ = self.rollback();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_guard_refuses_to_overwrite_external_modifications() {
        let base = std::env::temp_dir().join(format!(
            "am-skills-guard-test-{}-{}",
            std::process::id(),
            store::uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let link_path = base.join("test-link");
        let target1 = base.join("target1");
        let target2 = base.join("target2");
        let external_target = base.join("external");
        std::fs::create_dir_all(&target1).unwrap();
        std::fs::create_dir_all(&target2).unwrap();
        std::fs::create_dir_all(&external_target).unwrap();

        // 1. 外部重定向软链接：回滚必须报错且保留外部软链接，绝不覆盖
        atomic_create_or_replace_link(&target1, &link_path).unwrap();
        let mut guard1 = RollbackGuard::new();
        guard1.record_mutation(
            &link_path,
            None,
            ExpectedLinkState::Present(resolve_loose(&target1)),
        );
        atomic_create_or_replace_link(&external_target, &link_path).unwrap();
        assert!(guard1.rollback().is_err(), "遇到外部重定向时回滚必须报错");
        assert_eq!(
            resolve_link_target(&link_path).unwrap(),
            resolve_loose(&external_target),
            "遇到外部重定向时必须保留外部目标，绝不覆盖"
        );

        // 2. 外部将软链接替换为普通文件：回滚必须报错且保留外部文件，绝不覆盖
        let _ = std::fs::remove_file(&link_path);
        atomic_create_or_replace_link(&target1, &link_path).unwrap();
        let mut guard2 = RollbackGuard::new();
        guard2.record_mutation(
            &link_path,
            Some(resolve_loose(&target2)),
            ExpectedLinkState::Present(resolve_loose(&target1)),
        );
        let _ = std::fs::remove_file(&link_path);
        std::fs::write(&link_path, "external file content").unwrap();
        assert!(
            guard2.rollback().is_err(),
            "遇到外部非链接文件时回滚必须报错"
        );
        assert!(!is_link_entry(&link_path));
        assert_eq!(
            std::fs::read_to_string(&link_path).unwrap(),
            "external file content"
        );

        // 3. 删除操作后外部重新创建文件：回滚必须报错且保留外部文件，绝不覆盖
        let _ = std::fs::remove_file(&link_path);
        let mut guard3 = RollbackGuard::new();
        guard3.record_mutation(
            &link_path,
            Some(resolve_loose(&target2)),
            ExpectedLinkState::Absent,
        );
        std::fs::write(&link_path, "created externally while absent").unwrap();
        assert!(
            guard3.rollback().is_err(),
            "删除后外部重新创建时回滚必须报错"
        );
        assert_eq!(
            std::fs::read_to_string(&link_path).unwrap(),
            "created externally while absent"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rollback_guard_succeeds_when_no_tampering() {
        let base = std::env::temp_dir().join(format!(
            "am-skills-guard-clean-{}-{}",
            std::process::id(),
            store::uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let link_path = base.join("test-link");
        let target1 = base.join("target1");
        let target2 = base.join("target2");
        std::fs::create_dir_all(&target1).unwrap();
        std::fs::create_dir_all(&target2).unwrap();

        atomic_create_or_replace_link(&target1, &link_path).unwrap();
        let mut guard = RollbackGuard::new();
        guard.record_mutation(
            &link_path,
            Some(resolve_loose(&target2)),
            ExpectedLinkState::Present(resolve_loose(&target1)),
        );
        assert!(guard.rollback().is_ok(), "无外部干扰时回滚必须成功");
        assert_eq!(
            resolve_link_target(&link_path).unwrap(),
            resolve_loose(&target2),
            "无外部干扰时应成功恢复到旧目标"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
