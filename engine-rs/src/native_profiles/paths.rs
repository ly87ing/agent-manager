//! Account path isolation, including links in existing ancestors and file targets.
//!
//! Account identity paths never follow links. Shared skills and rules have their own
//! resource guards and are not identity files.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

use super::model::{self, ProfileDef, ProfileKind};
use crate::native::Context;
use crate::platform::Canon;
use crate::store;

const IDENTITY_FILES: [&str; 8] = [
    "settings.json",
    "settings.local.json",
    "config.toml",
    ".credentials.json",
    "auth.json",
    ".claude.json",
    "managed-settings.json",
    "managed-settings-cache.json",
];

fn is_link(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    false
}

/// HOME itself may use the OS's canonical alias (for example /var on macOS).
/// Every component below that trusted root must be an ordinary filesystem entry.
fn unaliased_home_path(home_dir: &Path, path: &Path, id: &str) -> Result<PathBuf> {
    let home = resolved(home_dir)?;
    let relative = path
        .strip_prefix(home_dir)
        .or_else(|_| path.strip_prefix(&home))
        .map_err(|_| anyhow!("[{id}] 账号路径必须位于可信用户主目录下"))?;
    let mut current = home;
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            bail!("[{id}] 账号路径含不明确的路径段");
        };
        current.push(name);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if is_link(&metadata) => {
                bail!("[{id}] 账号目录与身份文件不能使用符号链接、联接或重解析点");
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("[{id}] 无法确认账号路径安全性，拒绝操作"),
        }
    }
    resolved(&current)
}

fn check_identity_file(path: &Path, id: &str) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => bail!("[{id}] 无法确认账号身份文件安全性，拒绝操作"),
    };
    if is_link(&metadata) || !metadata.is_file() {
        bail!("[{id}] 账号身份文件必须是无链接的普通文件");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            bail!("[{id}] 账号身份文件存在硬链接别名，拒绝操作");
        }
    }
    #[cfg(windows)]
    {
        if windows_file_information(path)?.nNumberOfLinks != 1 {
            bail!("[{id}] 账号身份文件存在硬链接别名，拒绝操作");
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn windows_file_information(
    path: &Path,
) -> Result<windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // The handle remains open, and an unsuccessful call never reads the structure.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { info.assume_init() })
}

/// Resolve all existing ancestors, even when several final components do not exist.
/// A dangling link or an unreadable ancestor is an error, not a missing directory.
pub(super) fn resolved(path: &Path) -> Result<PathBuf> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(path.canon()?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| anyhow!("无法解析配置路径"))?;
            let name = path
                .file_name()
                .ok_or_else(|| anyhow!("无法解析配置路径"))?;
            Ok(resolved(parent)?.join(name))
        }
        Err(err) => Err(err.into()),
    }
}

/// Existing hard links share a file identity even though canonical path strings differ.
fn same_file(a: &Path, b: &Path) -> Result<bool> {
    let metadata = |path: &Path| match std::fs::metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    };
    let (Some(a_meta), Some(b_meta)) = (metadata(a)?, metadata(b)?) else {
        return Ok(false);
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(a_meta.dev() == b_meta.dev() && a_meta.ino() == b_meta.ino())
    }
    #[cfg(windows)]
    {
        let _ = (a_meta, b_meta);
        let identity = |path: &Path| -> Result<(u32, u32, u32)> {
            let info = windows_file_information(path)?;
            Ok((
                info.dwVolumeSerialNumber,
                info.nFileIndexHigh,
                info.nFileIndexLow,
            ))
        };
        Ok(identity(a)? == identity(b)?)
    }
}

fn starts_with(path: &Path, parent: &Path) -> bool {
    if path.starts_with(parent) {
        return true;
    }
    // Fail closed for case aliases on the normally case-insensitive Windows/macOS
    // filesystems, including names that do not exist yet. This deliberately also
    // rejects case-only account names on their case-sensitive volumes.
    if cfg!(any(windows, target_os = "macos")) {
        let mut path = path.components();
        return parent.components().all(|component| {
            path.next().is_some_and(|other| {
                component.as_os_str().to_string_lossy().to_lowercase()
                    == other.as_os_str().to_string_lossy().to_lowercase()
            })
        });
    }
    false
}

fn overlaps(a: &Path, b: &Path) -> bool {
    starts_with(a, b) || starts_with(b, a)
}

/// Validate the directory before any directory creation, CLI probing or file read.
pub(super) fn validate_config_dir(
    def: &ProfileDef,
    home_dir: &Path,
    data_dir: &Path,
) -> Result<PathBuf> {
    validate_config_dir_raw(&def.id, &def.config_dir_raw, home_dir, data_dir)
}

fn validate_config_dir_raw(
    id: &str,
    raw: &str,
    home_dir: &Path,
    data_dir: &Path,
) -> Result<PathBuf> {
    let untrimmed = raw;
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("[{id}] 配置目录不能为空");
    }
    if raw.chars().any(char::is_control) {
        bail!("[{id}] 配置目录含非法字符");
    }
    #[cfg(windows)]
    {
        // Normal Win32 paths discard trailing dots/spaces when creating components.
        // Reject that ambiguity before creating anything, including final whitespace
        // that the ordinary display/definition trimming below would otherwise hide.
        let untrimmed = model::expand_home(untrimmed.trim_start(), home_dir);
        if untrimmed.components().any(|component| {
            matches!(component, std::path::Component::Normal(name)
                if name.to_string_lossy().ends_with(['.', ' ']))
        }) {
            bail!("[{id}] Windows 配置目录的路径段不能以点或空格结尾");
        }
    }
    #[cfg(not(windows))]
    let _ = untrimmed;
    let path = model::expand_home(raw, home_dir);
    if !path.is_absolute() {
        bail!("[{id}] 配置目录必须是绝对路径或以 ~/ 开头");
    }
    if path.components().any(|c| c.as_os_str() == "..") {
        bail!("[{id}] 配置目录不允许包含 ..");
    }
    let real = unaliased_home_path(home_dir, &path, id)?;
    let home = resolved(home_dir)?;
    if !starts_with(&real, &home) || starts_with(&home, &real) {
        bail!("[{id}] 配置目录必须位于用户主目录下，不能是主目录本身");
    }
    for primary in [".claude", ".codex"] {
        if overlaps(&real, &resolved(&home_dir.join(primary))?) {
            bail!("[{id}] 配置目录不能与主实例目录重叠");
        }
    }
    if overlaps(&real, &resolved(data_dir)?) {
        bail!("[{id}] 配置目录不能与本工具的数据目录重叠");
    }
    if real.exists() && !real.is_dir() {
        bail!("[{id}] 配置目录已被一个同名文件占用");
    }
    Ok(real)
}

/// Both valid and disabled/malformed registrations reserve their usable directory.
/// Otherwise an invalid credential field could silently remove another account's protection.
fn other_accounts(ctx: &Context, policy: &Value, exclude_id: Option<&str>) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    if let Some(definitions) = policy
        .pointer("/agents/definitions")
        .and_then(Value::as_object)
    {
        for (id, raw) in definitions {
            if exclude_id == Some(id.as_str()) || raw.get("_delete") == Some(&Value::Bool(true)) {
                continue;
            }
            let Some(dir) = raw.pointer("/profile/config_dir").and_then(Value::as_str) else {
                continue;
            };
            let path = model::expand_home(dir.trim(), &ctx.home_dir);
            if !path.is_absolute()
                || dir.chars().any(char::is_control)
                || path.components().any(|c| c.as_os_str() == "..")
            {
                bail!("[{id}] 已登记账号的目录无法安全确定，请先修正定义");
            }
            paths.push(path);
        }
    }
    Ok(paths)
}

/// Shell command targets are never account configuration or manager data.
/// Ordinary dotfile links remain supported, but resolving a link is not ownership.
pub(crate) fn validate_shell_target(ctx: &Context, path: &Path, directory: bool) -> Result<()> {
    validate_non_account_target(ctx, path, directory, true)
}

/// Dependency programs may own manager data, but never account or MCP identity files.
pub(crate) fn validate_program_target(ctx: &Context, path: &Path, directory: bool) -> Result<()> {
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| is_link(&metadata)) {
        let parent = path.parent().ok_or_else(|| anyhow!("程序目标缺少父目录"))?;
        let location = resolved(parent)?.join(
            path.file_name()
                .ok_or_else(|| anyhow!("程序目标缺少名称"))?,
        );
        validate_non_account_location(ctx, &location, directory, false)?;
        // Moving a package temporarily leaves its launcher links dangling. Check
        // both the link's location and its destination without dereferencing it.
        let target = crate::platform::read_link(path)?;
        return validate_non_account_target(ctx, &parent.join(target), directory, false);
    }
    validate_non_account_target(ctx, path, directory, false)
}

pub(crate) fn validate_program_metadata_file(path: &Path) -> Result<()> {
    check_identity_file(path, "extension-program")
}

fn validate_non_account_target(
    ctx: &Context,
    path: &Path,
    directory: bool,
    protect_manager_directory: bool,
) -> Result<()> {
    let path = resolved(path)?;
    validate_non_account_location(ctx, &path, directory, protect_manager_directory)
}

fn validate_non_account_location(
    ctx: &Context,
    path: &Path,
    directory: bool,
    protect_manager_directory: bool,
) -> Result<()> {
    let policy = store::effective_policy(ctx)?;
    let mut directories = vec![ctx.home_dir.join(".claude"), ctx.home_dir.join(".codex")];
    if protect_manager_directory {
        directories.push(ctx.data_dir.clone());
    }
    directories.extend(other_accounts(ctx, &policy, None)?);
    let mut files = vec![
        ctx.home_dir.join(".claude.json"),
        ctx.data_dir.join("native-manager.db"),
    ];
    for target in crate::native_mcp::targets::agent_targets(&policy, &ctx.home_dir) {
        if !target.path.is_absolute() {
            bail!("已登记 MCP 目标无法安全确定，拒绝读写命令目标");
        }
        files.push(target.path);
    }
    for dir in &directories {
        files.extend(IDENTITY_FILES.map(|file| dir.join(file)));
        let protected = resolved(dir)?;
        if starts_with(&path, &protected) || (directory && starts_with(&protected, &path)) {
            bail!("命令目标与账号或本工具的数据目录冲突，拒绝读写");
        }
    }
    for file in files {
        let protected = resolved(&file)?;
        if same_file(&path, &protected)?
            || (starts_with(&path, &protected) && starts_with(&protected, &path))
            || (directory && starts_with(&protected, &path))
        {
            bail!("命令目标与账号身份文件或本工具的数据文件冲突，拒绝读写");
        }
    }
    Ok(())
}

fn protected(
    ctx: &Context,
    policy: &Value,
    def: &ProfileDef,
    path: &Path,
    directory: bool,
    write_target: bool,
) -> Result<()> {
    let mut directories = vec![
        ctx.home_dir.join(".claude"),
        ctx.home_dir.join(".codex"),
        ctx.data_dir.clone(),
    ];
    // Main configuration files may themselves live outside the main directories.
    let mut files = vec![
        ctx.home_dir.join(".claude.json"),
        ctx.data_dir.join("native-manager.db"),
    ];
    for main_dir in [".claude", ".codex"] {
        files.extend(IDENTITY_FILES.map(|file| ctx.home_dir.join(main_dir).join(file)));
    }
    // Authentication files are never account-settings write targets, even for this account.
    if write_target {
        let own_dir = model::expand_home(def.config_dir_raw.trim(), &ctx.home_dir);
        files.extend([".credentials.json", "auth.json"].map(|file| own_dir.join(file)));
    }
    for dir in other_accounts(ctx, policy, Some(&def.id))? {
        // Reserve known configuration/authentication files independently of malformed kind
        // or credential fields. They can be linked outside their account directory.
        files.extend(IDENTITY_FILES.map(|file| dir.join(file)));
        directories.push(dir);
    }
    for protected in directories {
        let protected = resolved(&protected)?;
        if starts_with(path, &protected)
            || (directory && (starts_with(&protected, path) || same_file(path, &protected)?))
        {
            bail!(
                "[{}] 实际配置路径与主实例、其他实例或本工具数据目录冲突，拒绝读写",
                def.id
            );
        }
    }
    for protected in files {
        let protected = resolved(&protected)?;
        if same_file(path, &protected)?
            || (directory && starts_with(&protected, path))
            || (starts_with(path, &protected) && starts_with(&protected, path))
        {
            bail!(
                "[{}] 实际配置目标与主实例、其他实例或本工具数据文件冲突，拒绝读写",
                def.id
            );
        }
    }
    Ok(())
}

pub(crate) fn config_dir(ctx: &Context, def: &ProfileDef) -> Result<PathBuf> {
    validate_account_dir(ctx, &def.id, def.kind, &def.config_dir_raw)
}

pub(crate) fn config_dir_in_policy(
    ctx: &Context,
    def: &ProfileDef,
    policy: &Value,
) -> Result<PathBuf> {
    let dir = model::validate_config_dir(def, &ctx.home_dir, &ctx.data_dir)?;
    protected(ctx, policy, def, &dir, true, false)?;
    for name in IDENTITY_FILES {
        let file = unaliased_home_path(&ctx.home_dir, &dir.join(name), &def.id)?;
        check_identity_file(&file, &def.id)?;
        protected(ctx, policy, def, &file, false, false)?;
    }
    Ok(dir)
}

fn path_definition(id: &str, kind: ProfileKind, raw: &str) -> ProfileDef {
    ProfileDef {
        id: id.to_string(),
        display_name: id.to_string(),
        base_agent: kind.as_str().to_string(),
        kind,
        config_dir_raw: raw.to_string(),
        env: serde_json::Map::new(),
        base_url: None,
        env_key: None,
        wire_api: None,
        model: None,
        enabled: true,
    }
}

/// Registering and executing an account use exactly the same directory/file boundary.
pub(crate) fn validate_account_dir(
    ctx: &Context,
    id: &str,
    kind: ProfileKind,
    raw: &str,
) -> Result<PathBuf> {
    config_dir_in_policy(
        ctx,
        &path_definition(id, kind, raw),
        &store::effective_policy(ctx)?,
    )
}

/// Validate the complete candidate, including collisions between newly imported accounts.
pub(crate) fn validate_policy_account_paths(ctx: &Context, policy: &Value) -> Result<()> {
    if let Some(definitions) = policy
        .pointer("/agents/definitions")
        .and_then(Value::as_object)
    {
        for (id, raw) in definitions {
            if raw.get("_delete") == Some(&Value::Bool(true)) {
                continue;
            }
            skills_dir_in_policy(ctx, policy, id)?;
            if raw.get("profile").is_some() {
                let dir = raw
                    .pointer("/profile/config_dir")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("[{id}] 账号定义缺少可用的配置目录"))?;
                config_dir_in_policy(ctx, &path_definition(id, ProfileKind::Claude, dir), policy)?;
            }
            if let Some(mcp) = raw.get("mcp_config") {
                let target = mcp
                    .get("path")
                    .unwrap_or(mcp)
                    .as_str()
                    .ok_or_else(|| anyhow!("[{id}] MCP 目标路径必须是字符串"))?;
                validate_account_resource_path(
                    ctx,
                    policy,
                    id,
                    &model::expand_home(target, &ctx.home_dir),
                )?;
            }
        }
    }
    Ok(())
}

/// Skill links may share their source, but their destination belongs to one account.
/// Resolve the same effective target for policy saves and every skill operation.
pub(crate) fn skills_dir_in_policy(ctx: &Context, policy: &Value, id: &str) -> Result<PathBuf> {
    let raw = policy
        .pointer("/agents/definitions")
        .and_then(|defs| defs.get(id))
        .ok_or_else(|| anyhow!("[{id}] 技能目标的账号定义不存在"))?;
    let expected = if raw.get("profile").is_some() {
        let dir = raw
            .pointer("/profile/config_dir")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("[{id}] 技能目标缺少账号配置目录"))?;
        Some(
            config_dir_in_policy(ctx, &path_definition(id, ProfileKind::Claude, dir), policy)?
                .join("skills"),
        )
    } else {
        match id {
            "claude-code" => Some(
                validate_primary_account_in_policy(ctx, ProfileKind::Claude, policy)?
                    .join("skills"),
            ),
            "codex" => Some(
                validate_primary_account_in_policy(ctx, ProfileKind::Codex, policy)?.join("skills"),
            ),
            _ => None,
        }
    };
    let configured = raw.get("skills_dir").or_else(|| {
        policy
            .get("skills")
            .and_then(|s| s.get("agents"))
            .and_then(|a| a.get(id))
    });
    let target = match configured {
        Some(value) => model::expand_home(
            value
                .as_str()
                .ok_or_else(|| anyhow!("[{id}] 技能目录必须是字符串"))?,
            &ctx.home_dir,
        ),
        None => expected
            .clone()
            .unwrap_or_else(|| ctx.home_dir.join(format!(".{id}/skills"))),
    };
    if !target.is_absolute()
        || target.to_string_lossy().chars().any(char::is_control)
        || target
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!("[{id}] 技能目录必须是无歧义的绝对路径");
    }
    #[cfg(windows)]
    if target.components().any(|c| matches!(c, std::path::Component::Normal(n) if n.to_string_lossy().ends_with(['.', ' ']))) {
        bail!("[{id}] Windows 技能目录的路径段不能以点或空格结尾");
    }
    let actual = if expected.is_some() || target.starts_with(&ctx.home_dir) {
        unaliased_home_path(&ctx.home_dir, &target, id)?
    } else {
        // Validate a missing child to inspect the destination as a directory,
        // including its ancestors, without following a directory link.
        crate::native_mcp::targets::validate_target_path(
            &target.join(".agent-manager-path-check"),
        )?;
        #[cfg(windows)]
        for ancestor in target.ancestors() {
            if std::fs::symlink_metadata(ancestor)
                .ok()
                .is_some_and(|m| is_link(&m))
            {
                bail!("[{id}] 技能目录不能使用联接或重解析点");
            }
        }
        resolved(&target)?
    };
    if actual.exists() && !actual.is_dir() {
        bail!("[{id}] 技能目录不是普通目录");
    }
    if let Some(expected) = expected {
        if actual != expected {
            bail!("[{id}] 技能目录未绑定本账号的 skills 目录，拒绝操作");
        }
    } else {
        protected(
            ctx,
            policy,
            &path_definition(id, ProfileKind::Claude, ""),
            &actual,
            true,
            false,
        )?;
        if overlaps(&actual, &resolved(&ctx.home_dir.join(".agents/skills"))?) {
            bail!("[{id}] 技能目录与 Codex 共用技能目录冲突，拒绝操作");
        }
    }
    // Keep the validated spelling used by existing skill adoption journals.
    // Canonical identity is used for validation, not to rename recorded paths.
    Ok(target)
}

/// Main Claude keeps its authentication record at HOME/.claude.json.
pub(crate) fn validate_primary_account(ctx: &Context, kind: ProfileKind) -> Result<PathBuf> {
    validate_primary_account_in_policy(ctx, kind, &store::effective_policy(ctx)?)
}

fn validate_primary_account_in_policy(
    ctx: &Context,
    kind: ProfileKind,
    policy: &Value,
) -> Result<PathBuf> {
    let id = kind.as_str();
    let dir = unaliased_home_path(&ctx.home_dir, &ctx.home_dir.join(format!(".{id}")), id)?;
    if dir.exists() && !dir.is_dir() {
        bail!("[{id}] 主账号配置目录不是目录");
    }
    let other_kind = if kind == ProfileKind::Claude {
        ".codex"
    } else {
        ".claude"
    };
    let mut boundaries = vec![ctx.data_dir.clone(), ctx.home_dir.join(other_kind)];
    let registered = other_accounts(ctx, policy, None)?;
    boundaries.extend(registered.iter().cloned());
    for boundary in &boundaries {
        let boundary = resolved(boundary)?;
        if overlaps(&dir, &boundary) || same_file(&dir, &boundary)? {
            bail!("[{id}] 主账号配置目录与其他账号或本工具目录冲突");
        }
    }
    let mut files = IDENTITY_FILES.map(|name| dir.join(name)).to_vec();
    if kind == ProfileKind::Claude {
        files.push(resolved(&ctx.home_dir)?.join(".claude.json"));
    }
    for path in files {
        let path = unaliased_home_path(&ctx.home_dir, &path, id)?;
        check_identity_file(&path, id)?;
        for boundary in &boundaries {
            let actual = resolved(boundary)?;
            if starts_with(&path, &actual) {
                bail!("[{id}] 主账号身份文件与其他账号或本工具目录冲突");
            }
        }
        for other_dir in registered
            .iter()
            .chain(std::iter::once(&ctx.home_dir.join(other_kind)))
        {
            for name in IDENTITY_FILES {
                let other = resolved(&other_dir.join(name))?;
                if path == other || same_file(&path, &other)? {
                    bail!("[{id}] 主账号身份文件与其他账号身份文件冲突");
                }
            }
        }
    }
    Ok(dir)
}

fn validate_mcp_format(raw: &Value, kind: ProfileKind, id: &str) -> Result<()> {
    let Some(format) = raw.pointer("/mcp_config/format") else {
        return Ok(());
    };
    let format = format
        .as_str()
        .ok_or_else(|| anyhow!("[{id}] MCP 格式必须是字符串"))?
        .to_ascii_lowercase();
    let valid = match kind {
        ProfileKind::Claude => matches!(format.as_str(), "json" | "claude-json"),
        ProfileKind::Codex => matches!(format.as_str(), "toml" | "codex-toml"),
    };
    if !valid {
        bail!("[{id}] MCP 文件格式与账号类型不一致，拒绝操作");
    }
    Ok(())
}

/// MCP shares account identity files, so its target must remain bound to that account.
/// Other assistant types cannot use MCP to read or mutate a protected account.
pub(crate) fn validate_account_resource_path(
    ctx: &Context,
    policy: &Value,
    agent_id: &str,
    target: &Path,
) -> Result<PathBuf> {
    let id = crate::native_mcp::model::canonicalize_agent_id(agent_id);
    let raw = policy
        .pointer("/agents/definitions")
        .and_then(|defs| defs.get(agent_id))
        .or_else(|| {
            policy
                .pointer("/agents/definitions")
                .and_then(|defs| defs.get(&id))
        })
        .ok_or_else(|| anyhow!("[{id}] 账号定义不存在，拒绝操作"))?;
    if raw.get("_delete") == Some(&Value::Bool(true)) {
        bail!("[{id}] 账号已删除，拒绝操作");
    }
    if raw.get("profile").is_some() {
        let def = model::parse_definition(&id, raw, true)?;
        validate_mcp_format(raw, def.kind, &id)?;
        let dir = config_dir_in_policy(ctx, &def, policy)?;
        let file = if def.kind == ProfileKind::Claude {
            ".claude.json"
        } else {
            "config.toml"
        };
        let actual = unaliased_home_path(&ctx.home_dir, target, &id)?;
        if actual != dir.join(file) {
            bail!("[{id}] MCP 目标未绑定本账号的身份文件，拒绝操作");
        }
        return target_in_policy(ctx, &def, &actual, policy);
    }
    let kind = match id.as_str() {
        "claude-code" => ProfileKind::Claude,
        "codex" => ProfileKind::Codex,
        _ => match raw
            .get("bin")
            .and_then(Value::as_str)
            .and_then(crate::native_agent_adapters::kind_for_bin)
        {
            Some(crate::native_agent_adapters::AdapterKind::Claude) => ProfileKind::Claude,
            Some(crate::native_agent_adapters::AdapterKind::Codex) => ProfileKind::Codex,
            None => {
                if !target.is_absolute()
                    || target.to_string_lossy().chars().any(char::is_control)
                    || target
                        .components()
                        .any(|component| matches!(component, std::path::Component::ParentDir))
                {
                    bail!("[{id}] MCP 路径必须是无歧义的绝对路径，拒绝操作");
                }
                #[cfg(windows)]
                if target.components().any(|component| {
                    matches!(component, std::path::Component::Normal(name)
                    if name.to_string_lossy().ends_with(['.', ' ']))
                }) {
                    bail!("[{id}] Windows MCP 路径段不能以点或空格结尾");
                }
                let home = resolved(&ctx.home_dir)?;
                let actual = if target.starts_with(&ctx.home_dir) || target.starts_with(&home) {
                    unaliased_home_path(&ctx.home_dir, target, &id)?
                } else {
                    crate::native_mcp::targets::validate_target_path(target)?;
                    #[cfg(windows)]
                    for component in target.ancestors() {
                        if std::fs::symlink_metadata(component)
                            .ok()
                            .is_some_and(|meta| is_link(&meta))
                        {
                            bail!("[{id}] MCP 路径不能使用联接或重解析点");
                        }
                    }
                    resolved(target)?
                };
                crate::native_mcp::targets::validate_target_path(&actual)?;
                check_identity_file(&actual, &id)?;
                protected(
                    ctx,
                    policy,
                    &path_definition(&id, ProfileKind::Claude, ""),
                    &actual,
                    false,
                    false,
                )?;
                return Ok(actual);
            }
        },
    };
    validate_mcp_format(raw, kind, &id)?;
    let dir = validate_primary_account_in_policy(ctx, kind, policy)?;
    let expected = if kind == ProfileKind::Claude {
        resolved(&ctx.home_dir)?.join(".claude.json")
    } else {
        dir.join("config.toml")
    };
    let actual = unaliased_home_path(&ctx.home_dir, target, &id)?;
    if actual != expected {
        bail!("[{id}] 主账号 MCP 目标未绑定主账号身份文件，拒绝操作");
    }
    check_identity_file(&actual, &id)?;
    Ok(actual)
}

/// Resolve and check the final file before reading it or deriving a write baseline.
pub(super) fn target(ctx: &Context, def: &ProfileDef, path: &Path) -> Result<PathBuf> {
    target_in_policy(ctx, def, path, &store::effective_policy(ctx)?)
}

pub(crate) fn target_in_policy(
    ctx: &Context,
    def: &ProfileDef,
    path: &Path,
    policy: &Value,
) -> Result<PathBuf> {
    let dir = config_dir_in_policy(ctx, def, policy)?;
    let real = unaliased_home_path(&ctx.home_dir, path, &def.id)?;
    if !starts_with(&real, &dir) || real == dir {
        bail!("[{}] 配置文件不属于该账号目录", def.id);
    }
    check_identity_file(&real, &def.id)?;
    protected(ctx, policy, def, &real, false, true)?;
    Ok(real)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Fixture {
        root: PathBuf,
        ctx: Context,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("am-path-guard-{}", store::uuid_v4()));
            let ctx = Context {
                home_dir: root.join("home"),
                data_dir: root.join("data"),
            };
            std::fs::create_dir_all(&ctx.home_dir).unwrap();
            Self { root, ctx }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn candidate_paths_validate_new_accounts_together_and_allow_tombstones() {
        let fixture = Fixture::new();
        let mut policy = json!({"agents":{"definitions":{
            "account-a":{"profile":{"config_dir":"~/independent"}},
            "account-b":{"profile":{"config_dir":"~/independent","env":"invalid"}},
        }}});
        assert!(validate_policy_account_paths(&fixture.ctx, &policy).is_err());
        assert!(!fixture.ctx.home_dir.join("independent").exists());
        policy["agents"]["definitions"]["account-b"]["_delete"] = json!(true);
        assert!(validate_policy_account_paths(&fixture.ctx, &policy).is_ok());
        assert!(
            !fixture.ctx.data_dir.exists(),
            "validation must not initialize storage"
        );
    }

    #[test]
    fn primary_guard_binds_distinct_claude_and_codex_paths_and_rejects_auth_hardlinks() {
        let fixture = Fixture::new();
        assert_eq!(
            validate_primary_account(&fixture.ctx, ProfileKind::Claude).unwrap(),
            resolved(&fixture.ctx.home_dir).unwrap().join(".claude")
        );
        assert_eq!(
            validate_primary_account(&fixture.ctx, ProfileKind::Codex).unwrap(),
            resolved(&fixture.ctx.home_dir).unwrap().join(".codex")
        );
        let original = fixture.root.join("dummy-auth.json");
        std::fs::write(&original, b"{}\n").unwrap();
        std::fs::hard_link(&original, fixture.ctx.home_dir.join(".claude.json")).unwrap();
        assert!(validate_primary_account(&fixture.ctx, ProfileKind::Claude).is_err());
        assert_eq!(std::fs::read(original).unwrap(), b"{}\n");
        assert!(!fixture.ctx.home_dir.join(".claude").exists());
    }

    #[test]
    fn mcp_targets_bind_profile_and_primary_identity_files_even_with_custom_executables() {
        let fixture = Fixture::new();
        let policy = json!({"agents":{"definitions":{
            "claude-code":{"bin":"/dummy/claude-fixture"},
            "codex":{"bin":"/dummy/codex-fixture"},
            "custom-claude":{"bin":"/dummy/claude.exe"},
            "account-a":{"bin":"claude","profile":{"base_agent":"claude-code","config_dir":"~/independent"}},
        }}});
        let home = &fixture.ctx.home_dir;
        assert!(validate_account_resource_path(
            &fixture.ctx,
            &policy,
            "claude-code",
            &home.join(".claude.json")
        )
        .is_ok());
        assert!(validate_account_resource_path(
            &fixture.ctx,
            &policy,
            "codex",
            &home.join(".codex/config.toml")
        )
        .is_ok());
        assert!(validate_account_resource_path(
            &fixture.ctx,
            &policy,
            "account-a",
            &home.join("independent/.claude.json")
        )
        .is_ok());
        for (id, path) in [
            ("claude-code", "independent/.claude.json"),
            ("codex", "independent/config.toml"),
            ("account-a", ".claude.json"),
            ("account-a", "independent/config.toml"),
            ("custom-claude", "independent/.claude.json"),
        ] {
            assert!(
                validate_account_resource_path(&fixture.ctx, &policy, id, &home.join(path))
                    .is_err(),
                "{id}: {path}"
            );
        }
        assert!(!home.join("independent").exists());
        assert!(!fixture.ctx.data_dir.exists());
    }

    #[test]
    fn candidate_policy_rejects_mcp_directory_overrides_without_reading_or_writing_them() {
        let fixture = Fixture::new();
        let mut policy = json!({"agents":{"definitions":{
            "account-a":{"bin":"claude","profile":{"base_agent":"claude-code","config_dir":"~/independent"},
                "mcp_config":{"path":"~/.claude.json","format":"json"}},
        }}});
        assert!(validate_policy_account_paths(&fixture.ctx, &policy).is_err());
        policy["agents"]["definitions"]["account-a"]["mcp_config"]["path"] =
            json!("~/independent/.claude.json");
        assert!(validate_policy_account_paths(&fixture.ctx, &policy).is_ok());
        policy["agents"]["definitions"]["account-a"]["mcp_config"]["format"] = json!("toml");
        assert!(validate_policy_account_paths(&fixture.ctx, &policy).is_err());
        policy["agents"]["definitions"]["account-a"]["mcp_config"]["format"] = json!("json");
        policy["agents"]["definitions"]["claude-code"] =
            json!({"bin":"/dummy/claude-fixture","mcp_config":{"path":"~/wrong.json"}});
        assert!(validate_policy_account_paths(&fixture.ctx, &policy).is_err());
        assert!(!fixture.ctx.home_dir.join("independent").exists());
        assert!(!fixture.ctx.home_dir.join("wrong.json").exists());
        assert!(!fixture.ctx.data_dir.exists());
    }

    #[test]
    fn unknown_mcp_assistants_cannot_target_main_or_registered_accounts() {
        let fixture = Fixture::new();
        let policy = json!({"agents":{"definitions":{
            "opencode":{"bin":"opencode"},
            "unknown":{"bin":"other-cli"},
            "disabled-account":{"profile":{"config_dir":"~/independent","env":"malformed"}},
        },"disabled-account":{"enabled":false}}});
        let home = &fixture.ctx.home_dir;
        for id in ["opencode", "unknown"] {
            assert!(validate_account_resource_path(
                &fixture.ctx,
                &policy,
                id,
                &fixture.root.join("external/opencode.json"),
            )
            .is_ok());
            assert!(validate_account_resource_path(
                &fixture.ctx,
                &policy,
                id,
                &home.join(".config/opencode/opencode.json"),
            )
            .is_ok());
            for target in [
                home.join(".claude.json"),
                home.join(".claude/settings.json"),
                home.join(".codex/config.toml"),
                home.join("independent/config.toml"),
                home.join("independent/arbitrary.json"),
                fixture.ctx.data_dir.join("native-manager.db"),
            ] {
                assert!(
                    validate_account_resource_path(&fixture.ctx, &policy, id, &target).is_err(),
                    "{id}: {}",
                    target.display()
                );
            }
        }
        std::fs::create_dir_all(home.join(".config/opencode")).unwrap();
        let own = home.join(".config/opencode/opencode.json");
        let main = home.join(".claude.json");
        std::fs::write(&main, b"{}\n").unwrap();
        std::fs::hard_link(&main, &own).unwrap();
        assert!(validate_account_resource_path(&fixture.ctx, &policy, "opencode", &own).is_err());
        assert_eq!(std::fs::read(&main).unwrap(), b"{}\n");
        assert!(!home.join("independent").exists());
        assert!(!fixture.ctx.data_dir.exists());
    }

    #[cfg(unix)]
    #[test]
    fn primary_guard_rejects_claude_home_login_link_and_other_account_link_to_main() {
        let fixture = Fixture::new();
        let auth = fixture.root.join("dummy-auth.json");
        std::fs::write(&auth, b"{}\n").unwrap();
        let main_auth = fixture.ctx.home_dir.join(".claude.json");
        std::os::unix::fs::symlink(&auth, &main_auth).unwrap();
        assert!(validate_primary_account(&fixture.ctx, ProfileKind::Claude).is_err());
        std::fs::remove_file(&main_auth).unwrap();
        std::fs::write(&main_auth, b"{}\n").unwrap();
        let other = fixture.ctx.home_dir.join("independent");
        std::fs::create_dir_all(&other).unwrap();
        std::os::unix::fs::symlink(&main_auth, other.join(".claude.json")).unwrap();
        let policy = json!({"agents":{"definitions":{"account-a":{"profile":{"config_dir":"~/independent"}}}}});
        store::write_policy(&store::open_write(&fixture.ctx).unwrap(), &policy).unwrap();
        assert!(validate_primary_account(&fixture.ctx, ProfileKind::Claude).is_err());
        assert_eq!(std::fs::read(&main_auth).unwrap(), b"{}\n");
        assert_eq!(std::fs::read(auth).unwrap(), b"{}\n");
    }
}
