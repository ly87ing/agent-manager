//! MCP 原子写入、回读校验与回滚保护

use anyhow::{anyhow, bail, Context as _, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::codecs::extract_entry;
use super::targets::{check_symlink_metadata, validate_target_path, Entry, Format};
use crate::store;

pub fn read_existing_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => {
            Err(anyhow::Error::from(err).context(format!("无法读取目标文件: {}", path.display())))
        }
    }
}

#[derive(Clone, Copy)]
enum ExpectedOld<'a> {
    Any,
    Exact(Option<&'a [u8]>),
}

fn write_temp_and_replace(
    path: &Path,
    bytes: &[u8],
    expected_old: ExpectedOld<'_>,
    mut before_compare: Option<&mut dyn FnMut(&Path) -> Result<()>>,
) -> Result<()> {
    validate_target_path(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("目标路径缺少父目录: {}", path.display()))?;
    let create_parent = matches!(expected_old, ExpectedOld::Any | ExpectedOld::Exact(None));
    if create_parent {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("无法创建配置目录 {}", parent.display()))?;
    } else if !parent.is_dir() {
        bail!("回滚目标父目录不存在，拒绝恢复: {}", parent.display());
    }

    let existing_meta = check_symlink_metadata(path)?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        existing_meta
            .as_ref()
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(0o600)
    };

    let file_stem = path.file_name().and_then(|s| s.to_str()).unwrap_or("tmp");
    let tmp_path = parent.join(format!(".{}.tmp_{}", file_stem, store::uuid_v4()));

    let write_res = (|| -> Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(mode);
        }
        let mut file = opts
            .open(&tmp_path)
            .with_context(|| format!("创建临时文件失败: {}", tmp_path.display()))?;

        file.write_all(bytes)
            .with_context(|| format!("写入临时文件失败: {}", tmp_path.display()))?;
        file.flush()
            .with_context(|| format!("刷新临时文件失败: {}", tmp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("同步临时文件失败: {}", tmp_path.display()))?;
        drop(file);

        validate_target_path(&tmp_path)?;

        if let Some(hook) = before_compare.as_mut() {
            hook(path)?;
        }
        if let ExpectedOld::Exact(expected) = expected_old {
            let current = read_existing_bytes(path)?;
            if current.as_deref() != expected {
                bail!(
                    "目标文件 {} 在临时文件准备后被外部修改，拒绝原子替换",
                    path.display()
                );
            }
        }

        std::fs::rename(&tmp_path, path)
            .with_context(|| format!("原子替换目标文件失败: {}", path.display()))?;

        #[cfg(unix)]
        {
            let parent_dir = std::fs::File::open(parent)
                .with_context(|| format!("无法打开目标父目录进行同步: {}", parent.display()))?;
            parent_dir
                .sync_all()
                .with_context(|| format!("无法同步目标父目录元数据: {}", parent.display()))?;
        }

        validate_target_path(path)?;
        Ok(())
    })();

    if let Err(write_err) = write_res {
        if let Err(cleanup_err) = std::fs::remove_file(&tmp_path) {
            if cleanup_err.kind() != std::io::ErrorKind::NotFound {
                return Err(write_err.context(format!(
                    "写入失败后清理临时文件 {} 同时失败: {cleanup_err:#}",
                    tmp_path.display()
                )));
            }
        }
        return Err(write_err);
    }
    Ok(())
}

pub fn safe_write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    write_temp_and_replace(path, bytes, ExpectedOld::Any, None)
}

pub fn safe_restore_bytes(path: &Path, old_bytes: Option<&[u8]>) -> Result<()> {
    validate_target_path(path)?;
    match old_bytes {
        Some(bytes) => safe_write_file(path, bytes),
        None => {
            if check_symlink_metadata(path)?.is_some() {
                validate_target_path(path)?;
                std::fs::remove_file(path)
                    .with_context(|| format!("回滚删除文件失败: {}", path.display()))?;
            }
            Ok(())
        }
    }
}

/// CAS 恢复：仅当目标当前字节与本次写入完全一致时才恢复 old_bytes
/// （old_bytes 为 None 表示该文件由本次事务创建，恢复即删除）；
/// 若写入后发生外部改动（内容不同或已被删除）则保留现状并返回明确冲突。
pub fn safe_restore_bytes_cas(
    path: &Path,
    written_bytes: &[u8],
    old_bytes: Option<&[u8]>,
) -> Result<()> {
    validate_target_path(path)?;
    match old_bytes {
        Some(bytes) => {
            write_temp_and_replace(path, bytes, ExpectedOld::Exact(Some(written_bytes)), None)
                .map_err(|err| anyhow!("回滚冲突: {err:#}"))
        }
        None => {
            if check_symlink_metadata(path)?.is_some() {
                validate_target_path(path)?;
            }
            let current = read_existing_bytes(path)?;
            if current.as_deref() != Some(written_bytes) {
                bail!(
                    "回滚冲突: 目标文件 {} 在写入后已被外部修改或删除，保留现状",
                    path.display()
                );
            }
            std::fs::remove_file(path)
                .with_context(|| format!("回滚删除文件失败: {}", path.display()))?;
            Ok(())
        }
    }
}

/// 原子写入 + 回读校验。`expected_old` 是渲染阶段读出的旧字节（None 表示
/// 渲染时目标不存在）：原子替换前再次读取并逐字节比较，不一致则
/// fail-closed 拒绝覆盖，保留外部新内容；回读校验失败时的恢复同样走 CAS，
/// 仅当当前字节仍等于本次写入时才恢复，否则保留并报告冲突。
pub fn atomic_write_checked(
    path: &Path,
    content: &str,
    expect: Option<(&str, &Entry, Format)>,
    expect_removed: Option<(&str, Format)>,
    expected_old: Option<&[u8]>,
) -> Result<()> {
    let content_bytes = content.as_bytes();
    write_temp_and_replace(path, content_bytes, ExpectedOld::Exact(expected_old), None)?;

    let verify = (|| -> Result<()> {
        if let Some((server, entry, format)) = expect {
            let (actual, managed) = extract_entry(path, format, server)?;
            let actual = actual.ok_or_else(|| anyhow!("回读校验失败: 条目 '{}' 缺失", server))?;
            if !managed {
                bail!("回读校验失败: 条目 '{}' 缺少受管标记", server);
            }
            if actual.digest() != entry.digest() {
                bail!("回读校验失败: 条目 '{}' 摘要与写入不一致", server);
            }
        }
        if let Some((server, format)) = expect_removed {
            let (actual, _) = extract_entry(path, format, server)?;
            if actual.is_some() {
                bail!("回读校验失败: 条目 '{}' 应已移除但仍存在", server);
            }
        }
        Ok(())
    })();

    if let Err(exc) = verify {
        if let Err(restore_err) = safe_restore_bytes_cas(path, content_bytes, expected_old) {
            return Err(exc.context(format!("回滚恢复失败: {restore_err:#}")));
        }
        return Err(exc);
    }
    Ok(())
}

pub struct Mutation {
    pub path: PathBuf,
    pub old_bytes: Option<Vec<u8>>,
    pub written_bytes: Vec<u8>,
}

pub struct RollbackGuard {
    pub mutations: Vec<Mutation>,
    pub committed: bool,
}

impl RollbackGuard {
    pub fn new() -> Self {
        Self {
            mutations: Vec::new(),
            committed: false,
        }
    }

    /// 只允许在一次成功写入之后登记：old_bytes 为渲染时读出的旧字节
    /// （None 表示目标原本不存在），written_bytes 为本次实际写入的字节。
    /// 同一路径在同一事务内重复写入时保留最初旧字节，CAS 基准更新为最新写入。
    pub fn record_written(
        &mut self,
        path: &Path,
        old_bytes: Option<Vec<u8>>,
        written_bytes: Vec<u8>,
    ) {
        if let Some(existing) = self.mutations.iter_mut().find(|m| m.path == path) {
            existing.written_bytes = written_bytes;
            return;
        }
        self.mutations.push(Mutation {
            path: path.to_path_buf(),
            old_bytes,
            written_bytes,
        });
    }

    pub fn commit(mut self) {
        self.committed = true;
    }

    pub fn rollback(&mut self) -> Result<()> {
        if self.committed {
            return Ok(());
        }
        let mut errors = Vec::new();
        for mutation in self.mutations.iter().rev() {
            if let Err(err) = safe_restore_bytes_cas(
                &mutation.path,
                &mutation.written_bytes,
                mutation.old_bytes.as_deref(),
            ) {
                errors.push(format!("{}: {err:#}", mutation.path.display()));
            }
        }
        if !errors.is_empty() {
            bail!("配置回滚失败或存在外部改动冲突: {}", errors.join("; "));
        }
        Ok(())
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

    fn test_root(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "am-native-mcp-rollback-unit-{}-{}",
            std::process::id(),
            tag
        ))
    }

    #[test]
    fn expected_old_is_checked_after_temp_sync_before_rename() {
        let root = test_root("cas-window");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("target.json");
        let original = b"original";
        let external = b"external";
        std::fs::write(&path, original).unwrap();

        let mut saw_prepared_temp = false;
        let mut inject_external_change = |target: &Path| -> Result<()> {
            saw_prepared_temp = std::fs::read_dir(&root).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".tmp_")
            });
            std::fs::write(target, external).map_err(Into::into)
        };
        let result = write_temp_and_replace(
            &path,
            b"replacement",
            ExpectedOld::Exact(Some(original)),
            Some(&mut inject_external_change),
        );

        assert!(result.is_err(), "临时文件准备后发生外部变化必须拒绝 rename");
        assert!(
            saw_prepared_temp,
            "测试 seam 必须发生在 temp 写入并 sync 之后"
        );
        assert_eq!(std::fs::read(&path).unwrap(), external);
        assert!(
            std::fs::read_dir(&root).unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp_")),
            "CAS 拒绝后必须清理临时文件"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rollback_guard_registration_is_memory_only_and_keeps_first_old() {
        let path = Path::new("/path/that/need/not/exist");
        let mut guard = RollbackGuard::new();
        guard.record_written(path, None, b"first".to_vec());
        guard.record_written(path, Some(b"ignored".to_vec()), b"latest".to_vec());

        assert_eq!(guard.mutations.len(), 1);
        assert!(guard.mutations[0].old_bytes.is_none());
        assert_eq!(guard.mutations[0].written_bytes, b"latest");
    }
}
