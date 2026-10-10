//! 凭据取值只来自用户显式选择的外部 `.env` 文件或进程环境。
//!
//! `native-manager.db` 只保存外部文件路径，不保存凭据值。外部文件优先于进程环境，
//! 选择外部文件后，它是唯一来源；文件失败或缺值绝不回退父进程中的同名变量。

use crate::platform::Canon;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use rusqlite::Connection;

use crate::native::Context;

#[derive(Clone, Default)]
struct Snapshot {
    values: BTreeMap<String, String>,
    configured_path: Option<String>,
    error: Option<String>,
}

thread_local! {
    static CACHE: RefCell<Snapshot> = RefCell::new(Snapshot::default());
}

/// 变量名规则：与 `${VAR}` 占位符可用的形状一致。
pub fn valid_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn configured_path(ctx: &Context) -> Result<Option<(String, PathBuf)>> {
    let raw = crate::store::effective_settings(ctx)?.secrets_file_path;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let path = if trimmed == "~" {
        ctx.home_dir.clone()
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        ctx.home_dir.join(rest)
    } else {
        PathBuf::from(trimmed)
    };
    if !path.is_absolute() {
        bail!("外部密钥文件必须使用绝对路径或 ~/ 开头的路径");
    }
    Ok(Some((trimmed.to_string(), path)))
}

#[cfg(unix)]
fn validate_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(path)?.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        bail!("外部密钥文件权限过宽，请先执行 chmod 600");
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn validate_permissions(path: &Path) -> Result<()> {
    validate_windows_acl(path, true)
}

/// Lock ancestors may be publicly readable, but another user must not be able
/// to replace their children. Secret sources and runtime locks remain private.
#[cfg(any(windows, test))]
fn windows_acl_grants_untrusted_access(mask: u32, private: bool, root: bool) -> bool {
    if private {
        return mask != 0;
    }
    // A volume/share root may let users add new children. This does not permit
    // replacing an existing, independently validated ancestor. Delete-child,
    // deletion, ACL/owner changes and generic mutation rights still fail closed.
    let mutation = if root { 0x0000_0150 } else { 0x0000_0156 };
    mask & (mutation | 0x000D_0000 | 0x5000_0000) != 0
}

#[cfg(any(windows, test))]
fn windows_acl_allows_installer_ancestor_owner(sid: &[u8], private: bool) -> bool {
    // Windows Modules Installer: S-1-5-80-956008885-3418522649-1831038044-
    // 1853292631-2271478464. This exception is only for ancestor ownership;
    // it never trusts another service SID or any private-file/ACL grant.
    const SUBAUTHORITIES: [u32; 6] = [
        80, 956008885, 3418522649, 1831038044, 1853292631, 2271478464,
    ];
    !private
        && sid.len() == 32
        && sid[..8] == [1, 6, 0, 0, 0, 0, 0, 5]
        && sid[8..]
            .chunks_exact(4)
            .zip(SUBAUTHORITIES)
            .all(|(bytes, value)| bytes == value.to_le_bytes())
}

#[cfg(windows)]
pub(crate) fn validate_windows_acl(path: &Path, private: bool) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        Security::*,
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    // Keep std's verbatim Windows prefix: raw Win32 ACL APIs otherwise reject
    // existing paths beyond MAX_PATH even though Rust's file APIs opened them.
    let native_path =
        std::fs::canonicalize(path).with_context(|| "无法解析 Windows ACL 路径，拒绝操作")?;
    let wide: Vec<u16> = native_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // Read-only OS APIs; unknown ACL shapes are rejected instead of being guessed safe.
    unsafe {
        let mut needed = 0;
        GetFileSecurityW(
            wide.as_ptr(),
            DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            0,
            &mut needed,
        );
        if needed == 0 || needed > 64 * 1024 {
            bail!("无法可靠读取外部密钥文件 Windows ACL，拒绝操作");
        }
        let mut descriptor = vec![0u8; needed as usize];
        if GetFileSecurityW(
            wide.as_ptr(),
            DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
            descriptor.as_mut_ptr().cast(),
            needed,
            &mut needed,
        ) == 0
        {
            bail!("无法读取外部密钥文件 Windows ACL，拒绝操作");
        }
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            bail!("无法确认当前 Windows 用户，拒绝操作");
        }
        let mut token_size = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut token_size);
        if token_size == 0 || token_size > 64 * 1024 {
            CloseHandle(token);
            bail!("无法读取当前 Windows 用户，拒绝操作");
        }
        let mut token_bytes = vec![0u8; token_size as usize];
        let read_ok = GetTokenInformation(
            token,
            TokenUser,
            token_bytes.as_mut_ptr().cast(),
            token_size,
            &mut token_size,
        );
        CloseHandle(token);
        if read_ok == 0 {
            bail!("无法读取当前 Windows 用户，拒绝操作");
        }
        let user = std::ptr::read_unaligned(token_bytes.as_ptr().cast::<TOKEN_USER>())
            .User
            .Sid;
        let mut system = [0u8; 68];
        let mut admins = [0u8; 68];
        let mut system_len = system.len() as u32;
        let mut admins_len = admins.len() as u32;
        if CreateWellKnownSid(
            WinLocalSystemSid,
            std::ptr::null_mut(),
            system.as_mut_ptr().cast(),
            &mut system_len,
        ) == 0
            || CreateWellKnownSid(
                WinBuiltinAdministratorsSid,
                std::ptr::null_mut(),
                admins.as_mut_ptr().cast(),
                &mut admins_len,
            ) == 0
        {
            bail!("无法确认 Windows 系统权限，拒绝操作");
        }
        let mut trusted = |sid| {
            IsValidSid(sid) != 0
                && (EqualSid(sid, user) != 0
                    || EqualSid(sid, system.as_mut_ptr().cast()) != 0
                    || EqualSid(sid, admins.as_mut_ptr().cast()) != 0)
        };
        let mut owner = std::ptr::null_mut();
        let mut defaulted = 0;
        if GetSecurityDescriptorOwner(descriptor.as_mut_ptr().cast(), &mut owner, &mut defaulted)
            == 0
            || IsValidSid(owner) == 0
            || (!trusted(owner)
                && !windows_acl_allows_installer_ancestor_owner(
                    std::slice::from_raw_parts(owner.cast::<u8>(), GetLengthSid(owner) as usize),
                    private,
                ))
        {
            bail!("外部密钥文件所有者不是当前用户或系统管理员，拒绝操作");
        }
        let mut acl = std::ptr::null_mut();
        let mut present = 0;
        if GetSecurityDescriptorDacl(
            descriptor.as_mut_ptr().cast(),
            &mut present,
            &mut acl,
            &mut defaulted,
        ) == 0
            || present == 0
            || acl.is_null()
        {
            bail!("外部密钥文件没有受限 Windows ACL，拒绝操作");
        }
        let mut info: ACL_SIZE_INFORMATION = std::mem::zeroed();
        if GetAclInformation(
            acl,
            (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        ) == 0
        {
            bail!("外部密钥文件 Windows ACL 无法识别，拒绝操作");
        }
        for index in 0..info.AceCount {
            let mut raw = std::ptr::null_mut();
            if GetAce(acl, index, &mut raw) == 0 || raw.is_null() {
                bail!("外部密钥文件 Windows ACL 无法识别，拒绝操作");
            }
            let header = std::ptr::read_unaligned(raw.cast::<ACE_HEADER>());
            if u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0 || header.AceType == 1 {
                continue;
            }
            if header.AceType != 0
                || usize::from(header.AceSize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
            {
                bail!("外部密钥文件 Windows ACL 含无法可靠判断的授权，拒绝操作");
            }
            let ace = raw.cast::<ACCESS_ALLOWED_ACE>();
            let mask = std::ptr::read_unaligned(std::ptr::addr_of!((*ace).Mask));
            let sid = std::ptr::addr_of_mut!((*ace).SidStart).cast();
            let dangerous =
                windows_acl_grants_untrusted_access(mask, private, path.parent().is_none());
            if dangerous && !trusted(sid) {
                bail!("外部密钥文件 Windows ACL 允许其他用户访问，请先限制权限");
            }
        }
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn validate_permissions(_path: &Path) -> Result<()> {
    bail!("当前平台无法可靠确认外部密钥文件权限，拒绝操作")
}

fn validated_source_path(ctx: &Context) -> Result<(String, PathBuf)> {
    let Some((display, path)) = configured_path(ctx)? else {
        bail!("尚未选择外部密钥文件，请先到“设置 → 密钥”配置");
    };
    let metadata = fs::symlink_metadata(&path).with_context(|| "外部密钥文件不存在或无法读取")?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("外部密钥文件必须是普通文件，不能是目录或符号链接");
    }
    validate_permissions(&path)?;
    if metadata.len() > 1024 * 1024 {
        bail!("外部密钥文件超过安全读取上限");
    }
    let canonical = path.canon().with_context(|| "无法解析外部密钥文件路径")?;
    if let Ok(data_dir) = ctx.data_dir.canon() {
        if canonical.starts_with(data_dir) {
            bail!("外部密钥文件不能放在 Agent Manager 数据目录中");
        }
    }
    Ok((display, canonical))
}

/// 新建一个空的外部密钥文件（`secrets:source --create`）：文件已存在就什么都不动；不存在时
/// 缺的父目录以 0700 新建，文件以 0600 独占新建（不跟随、不覆盖）。不能建在数据目录里。
/// 返回这次新建的文件路径。
pub fn create_source_file(ctx: &Context, raw: &str) -> Result<Option<PathBuf>> {
    let trimmed = raw.trim();
    let path = if let Some(rest) = trimmed.strip_prefix("~/") {
        ctx.home_dir.join(rest)
    } else {
        PathBuf::from(trimmed)
    };
    if !path.is_absolute() || trimmed == "~" {
        bail!("外部密钥文件必须使用绝对路径或 ~/ 开头的文件路径");
    }
    if path.starts_with(&ctx.data_dir) {
        bail!("外部密钥文件不能放在 Agent Manager 数据目录中");
    }
    if fs::symlink_metadata(&path).is_ok() {
        return Ok(None);
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("外部密钥文件路径没有上级目录"))?;
    if !parent.exists() {
        fs::create_dir_all(parent).with_context(|| "无法创建外部密钥文件所在的目录")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        }
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .with_context(|| "无法新建外部密钥文件")?;
    file.write_all("# Agent Manager 保存的 API Key：一行一个 KEY=VALUE，仅本人可读\n".as_bytes())?;
    file.sync_all()?;
    Ok(Some(path))
}

pub fn validate_source(ctx: &Context, raw: &str) -> Result<()> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let path = if trimmed == "~" {
        ctx.home_dir.clone()
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        ctx.home_dir.join(rest)
    } else {
        PathBuf::from(trimmed)
    };
    if !path.is_absolute() {
        bail!("外部密钥文件必须使用绝对路径或 ~/ 开头的路径");
    }
    let metadata = fs::symlink_metadata(&path).with_context(|| "外部密钥文件不存在或无法读取")?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("外部密钥文件必须是普通文件，不能是目录或符号链接");
    }
    validate_permissions(&path)?;
    if metadata.len() > 1024 * 1024 {
        bail!("外部密钥文件超过安全读取上限");
    }
    let canonical = path.canon().with_context(|| "无法解析外部密钥文件路径")?;
    if let Ok(data_dir) = ctx.data_dir.canon() {
        if canonical.starts_with(data_dir) {
            bail!("外部密钥文件不能放在 Agent Manager 数据目录中");
        }
    }
    let text = read_secret_text(&path)?;
    parse_env(&text)?;
    Ok(())
}

pub(crate) fn source_identity(ctx: &Context, raw: &str) -> Result<Option<(PathBuf, Vec<u8>)>> {
    validate_source(ctx, raw)?;
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let path = if raw == "~" {
        ctx.home_dir.clone()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        ctx.home_dir.join(rest)
    } else {
        PathBuf::from(raw)
    };
    Ok(Some((path.canon()?, read_secret_bytes(&path)?)))
}

const MAX_SECRET_BYTES: u64 = 1024 * 1024;

fn read_secret_bytes(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .with_context(|| "外部密钥文件不可读取")?
        .take(MAX_SECRET_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| "外部密钥文件不可读取")?;
    if bytes.len() as u64 > MAX_SECRET_BYTES {
        bail!("外部密钥文件超过安全读取上限");
    }
    Ok(bytes)
}

fn read_secret_text(path: &Path) -> Result<String> {
    String::from_utf8(read_secret_bytes(path)?).with_context(|| "外部密钥文件不是可读的 UTF-8 文本")
}

fn parse_value(raw: &str) -> Result<String> {
    let value = raw.trim();
    if value.starts_with('"') {
        return serde_json::from_str::<String>(value)
            .map_err(|_| anyhow::anyhow!("双引号取值格式不正确"));
    }
    if value.starts_with('\'') {
        if value.len() < 2 || !value.ends_with('\'') {
            bail!("单引号取值格式不正确");
        }
        return Ok(value[1..value.len() - 1].to_string());
    }
    Ok(value.to_string())
}

fn key_of_line(line: &str) -> Option<&str> {
    let mut item = line.trim();
    if item.is_empty() || item.starts_with('#') {
        return None;
    }
    if let Some(rest) = item.strip_prefix("export ") {
        item = rest.trim_start();
    }
    let (key, _) = item.split_once('=')?;
    let key = key.trim();
    valid_var_name(key).then_some(key)
}

fn parse_env(text: &str) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    let mut case_names = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        let mut item = line.trim();
        if item.is_empty() || item.starts_with('#') {
            continue;
        }
        if let Some(rest) = item.strip_prefix("export ") {
            item = rest.trim_start();
        }
        let Some((key, raw)) = item.split_once('=') else {
            bail!("外部密钥文件第 {} 行不是 KEY=VALUE", index + 1);
        };
        let key = key.trim();
        if !valid_var_name(key) {
            bail!("外部密钥文件第 {} 行的变量名不合法", index + 1);
        }
        let value =
            parse_value(raw).with_context(|| format!("外部密钥文件第 {} 行取值无效", index + 1))?;
        if value.contains('\0') {
            bail!("外部密钥文件第 {} 行含非法字符", index + 1);
        }
        if !case_names.insert(key.to_ascii_uppercase())
            || values.insert(key.to_string(), value).is_some()
        {
            bail!("外部密钥文件中存在重复变量 {key}");
        }
    }
    Ok(values)
}

fn load_snapshot(ctx: &Context) -> Snapshot {
    let configured = match configured_path(ctx) {
        Ok(value) => value,
        Err(err) => {
            return Snapshot {
                error: Some(format!("{err:#}")),
                ..Snapshot::default()
            };
        }
    };
    let Some((display, _)) = configured else {
        return Snapshot::default();
    };
    match validated_source_path(ctx).and_then(|(_, path)| {
        let text = read_secret_text(&path)?;
        parse_env(&text)
    }) {
        Ok(values) => Snapshot {
            values,
            configured_path: Some(display),
            error: None,
        },
        Err(err) => Snapshot {
            values: BTreeMap::new(),
            configured_path: Some(display),
            error: Some(format!("{err:#}")),
        },
    }
}

/// 每条命令开头重新读取外部文件；用户修改后无需重启应用。
pub fn refresh(ctx: &Context) {
    CACHE.with(|cache| *cache.borrow_mut() = load_snapshot(ctx));
}

pub fn lookup(name: &str) -> Option<String> {
    CACHE.with(|cache| {
        let snapshot = cache.borrow();
        if snapshot.configured_path.is_some() || snapshot.error.is_some() {
            if snapshot.error.is_some() {
                return None;
            }
            return snapshot
                .values
                .get(name)
                .filter(|value| !value.is_empty())
                .cloned();
        }
        std::env::var(name).ok().filter(|value| !value.is_empty())
    })
}

/// A configured credential source must remain valid even when a caller supplies a pending value.
pub(crate) fn ensure_source_ready() -> Result<()> {
    let (configured, error) = CACHE.with(|cache| {
        let snapshot = cache.borrow();
        (snapshot.configured_path.is_some(), snapshot.error.is_some())
    });
    if error {
        bail!("已选择的外部密钥来源不可用，拒绝操作；不会回退进程环境");
    }
    if !configured {
        bail!("账号凭据必须来自显式选择的外部密钥来源；请先配置来源，不会继承进程环境");
    }
    Ok(())
}

/// Controlled account operations never inherit credentials from the parent process.
pub(crate) fn lookup_account(name: &str) -> Result<String> {
    ensure_source_ready()?;
    CACHE
        .with(|cache| {
            cache
                .borrow()
                .values
                .get(name)
                .filter(|value| !value.is_empty())
                .cloned()
        })
        .ok_or_else(|| anyhow::anyhow!("账号独占密钥变量未在已选择的外部来源中设置"))
}

pub fn is_set(name: &str) -> bool {
    lookup(name).is_some()
}

/// 变量是否来自已选择的外部密钥文件。
pub fn is_stored(name: &str) -> bool {
    CACHE.with(|cache| {
        cache
            .borrow()
            .values
            .get(name)
            .is_some_and(|value| !value.is_empty())
    })
}

pub fn source_state() -> (Option<String>, Option<String>) {
    CACHE.with(|cache| {
        let snapshot = cache.borrow();
        (snapshot.configured_path.clone(), snapshot.error.clone())
    })
}

/// 外部密钥文件里已有的变量名（含空值）；只返回名字。
pub fn stored_names() -> BTreeSet<String> {
    CACHE.with(|cache| cache.borrow().values.keys().cloned().collect())
}

fn render_line(name: &str, value: &str) -> Result<String> {
    Ok(format!("{name}={}", serde_json::to_string(value)?))
}

fn render_updated(text: &str, updates: &BTreeMap<String, Option<String>>) -> Result<String> {
    let current = parse_env(text)?;
    if updates.keys().any(|name| {
        current
            .keys()
            .any(|old| old != name && old.eq_ignore_ascii_case(name))
    }) {
        bail!("密钥变量名称存在大小写别名，拒绝更改或隐式重命名");
    }
    let mut seen = BTreeSet::new();
    let mut lines = Vec::new();
    for line in text.lines() {
        if let Some(key) = key_of_line(line) {
            if let Some(value) = updates.get(key) {
                seen.insert(key.to_string());
                if let Some(value) = value {
                    lines.push(render_line(key, value)?);
                }
                continue;
            }
        }
        lines.push(line.to_string());
    }
    for (key, value) in updates {
        if !seen.contains(key) {
            if let Some(value) = value {
                lines.push(render_line(key, value)?);
            }
        }
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    Ok(out)
}

fn atomic_write_checked(path: &Path, text: &str, expected: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("外部密钥文件没有父目录"))?;
    let temp = parent.join(format!(
        ".agent-manager-secrets-{}-{}.tmp",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<()> {
        let mut file = options.open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || read_secret_bytes(path)? != expected
        {
            bail!("外部密钥来源已变化，拒绝覆盖外部修改");
        }
        validate_permissions(path)?;
        validate_permissions(&temp)?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.with_context(|| "写入外部密钥文件失败")
}

/// A source-file change staged without writes; callers coordinate it with account configuration.
pub(crate) struct SecretWrite {
    path: PathBuf,
    old: Vec<u8>,
    new: Vec<u8>,
}

pub(crate) fn plan_set(ctx: &Context, name: &str, value: &str) -> Result<SecretWrite> {
    if !valid_var_name(name) || value.is_empty() || value.chars().any(char::is_control) {
        bail!("账号密钥变量或取值格式不正确");
    }
    plan_change(ctx, name, value)
}

pub(crate) fn plan_change(ctx: &Context, name: &str, value: &str) -> Result<SecretWrite> {
    if !valid_var_name(name) || value.contains('\0') {
        bail!("密钥变量或取值格式不正确");
    }
    let (_, path) = validated_source_path(ctx)?;
    let old = read_secret_bytes(&path)?;
    if old.len() > 1024 * 1024 {
        bail!("外部密钥文件超过安全读取上限");
    }
    let text = std::str::from_utf8(&old).with_context(|| "外部密钥文件不是 UTF-8 文本")?;
    parse_env(text)?;
    let updates = BTreeMap::from([(
        name.to_string(),
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        },
    )]);
    let new = render_updated(text, &updates)?.into_bytes();
    if new.len() > 1024 * 1024 {
        bail!("外部密钥文件超过安全写入上限");
    }
    Ok(SecretWrite { path, old, new })
}

impl SecretWrite {
    pub(crate) fn validate(&self, ctx: &Context) -> Result<()> {
        let (_, current) = validated_source_path(ctx)?;
        if current != self.path || read_secret_bytes(&self.path)? != self.old {
            bail!("外部密钥来源已变化，没有写入；请重新预览");
        }
        Ok(())
    }

    pub(crate) fn commit(&self, ctx: &Context) -> Result<()> {
        self.validate(ctx)?;
        atomic_write_checked(&self.path, std::str::from_utf8(&self.new)?, &self.old)
    }

    pub(crate) fn verify_committed(&self, ctx: &Context) -> Result<()> {
        let (_, current) = validated_source_path(ctx)?;
        if current != self.path || read_secret_bytes(&self.path)? != self.new {
            bail!("外部密钥来源写后已变化，拒绝提交账号变更");
        }
        Ok(())
    }

    pub(crate) fn rollback(&self, ctx: &Context) -> Result<()> {
        let (_, current) = validated_source_path(ctx)?;
        if current != self.path || read_secret_bytes(&self.path)? != self.new {
            bail!("外部密钥来源已被再次修改，拒绝覆盖外部修改");
        }
        atomic_write_checked(&self.path, std::str::from_utf8(&self.old)?, &self.new)
    }
}

/// 只补缺失值的合并结果：同名不同值时拒绝；没有要补的返回 None。不落盘。
fn plan_merge_named(
    ctx: &Context,
    incoming: &BTreeMap<String, String>,
) -> Result<(SecretWrite, Vec<String>)> {
    let (_, path) = validated_source_path(ctx)?;
    let text = read_secret_text(&path)?;
    let current = parse_env(&text)?;
    let mut updates = BTreeMap::new();
    let mut merged = Vec::new();
    for (name, value) in incoming {
        if !valid_var_name(name) {
            bail!("旧凭据变量名不合法，拒绝迁移或丢弃原记录");
        }
        match current.get(name) {
            Some(existing) if existing.is_empty() => {
                updates.insert(name.clone(), Some(value.clone()));
            }
            Some(existing) if existing != value => {
                bail!("外部密钥文件中的变量 {name} 已存在且取值不同，拒绝覆盖")
            }
            Some(_) => {}
            None => {
                updates.insert(name.clone(), Some(value.clone()));
            }
        }
        merged.push(name.clone());
    }
    merged.sort();
    merged.dedup();
    let new = if updates.is_empty() {
        text.as_bytes().to_vec()
    } else {
        render_updated(&text, &updates)?.into_bytes()
    };
    if new.len() as u64 > MAX_SECRET_BYTES {
        bail!("外部密钥文件超过安全写入上限");
    }
    Ok((
        SecretWrite {
            path,
            old: text.into_bytes(),
            new,
        },
        merged,
    ))
}

pub fn legacy_count(ctx: &Context) -> Result<usize> {
    let Some(conn) = crate::store::open_readonly(ctx)? else {
        return Ok(0);
    };
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'secret_values'",
        [],
        |row| row.get(0),
    )?;
    if exists == 0 {
        return Ok(0);
    }
    Ok(
        conn.query_row("SELECT COUNT(*) FROM secret_values", [], |row| {
            row.get::<_, i64>(0)
        })? as usize,
    )
}

fn read_legacy(conn: &Connection) -> Result<BTreeMap<String, String>> {
    let mut stmt = conn.prepare("SELECT name, value FROM secret_values ORDER BY name")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (name, value) = row?;
        out.insert(name, value);
    }
    Ok(out)
}

/// Explicit migration holds a DB transaction and restores its own source write on failure.
/// Account locks and reference ownership are checked by the command entry point.
pub fn migrate_legacy(ctx: &Context) -> Result<Vec<String>> {
    if legacy_count(ctx)? == 0 {
        return Ok(Vec::new());
    }
    let Some(conn) = crate::store::open_readonly(ctx)? else {
        return Ok(Vec::new());
    };
    let values = read_legacy(&conn)?;
    drop(conn);
    for name in values.keys() {
        crate::native_secrets::validate_variable_owners(ctx, name, None)?;
    }
    let (write, migrated) = plan_merge_named(ctx, &values)?;
    let mut conn = crate::store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if read_legacy(&tx)? != values {
        bail!("旧凭据已变化，没有迁移；请重新检查");
    }
    let owners = crate::native_secrets::account_secret_owners_in_policy(
        ctx,
        &crate::store::policy_from_connection(&tx)?,
    )?;
    for name in values.keys() {
        let ids = owners
            .get(&name.to_ascii_uppercase())
            .cloned()
            .unwrap_or_default();
        if ids.len() > 1 || ids.iter().any(|id| id.starts_with("retired:")) {
            bail!("旧凭据归属冲突或已退役，未迁移任何值");
        }
        if let Some(account) = ids.iter().next() {
            crate::native_secrets::claim_variable_ownership(
                ctx,
                &tx,
                account,
                &std::collections::BTreeSet::from([name.clone()]),
            )?;
        }
    }
    write.validate(ctx)?;
    tx.execute("DROP TABLE IF EXISTS secret_values", [])?;
    let changed = write.old != write.new;
    if changed {
        write.commit(ctx)?;
    }
    let result = write
        .verify_committed(ctx)
        .and_then(|()| tx.commit().map_err(Into::into));
    if let Err(err) = result {
        if changed {
            write.rollback(ctx).map_err(|_| {
                anyhow::anyhow!("凭据迁移未提交，且外部来源发生并发变化；拒绝覆盖，请检查来源文件")
            })?;
        }
        return Err(err.context("凭据迁移未提交，来源文件已恢复"));
    }
    Ok(migrated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_installer_owner_exception_excludes_private_files_and_other_services() {
        let mut installer = vec![1, 6, 0, 0, 0, 0, 0, 5];
        for value in [
            80u32, 956008885, 3418522649, 1831038044, 1853292631, 2271478464,
        ] {
            installer.extend(value.to_le_bytes());
        }
        assert!(windows_acl_allows_installer_ancestor_owner(
            &installer, false
        ));
        assert!(!windows_acl_allows_installer_ancestor_owner(
            &installer, true
        ));
        for offset in [0, 1, 7, 8, 12, 16, 20, 24, 28] {
            let mut other_service = installer.clone();
            other_service[offset] ^= 1;
            assert!(!windows_acl_allows_installer_ancestor_owner(
                &other_service,
                false
            ));
        }
        assert!(!windows_acl_allows_installer_ancestor_owner(
            &installer[..28],
            false
        ));
        installer.extend(0u32.to_le_bytes());
        assert!(!windows_acl_allows_installer_ancestor_owner(
            &installer, false
        ));
    }

    #[test]
    fn windows_root_acl_allows_new_children_but_rejects_replacement_rights() {
        let read_traverse = 0x0012_00A9;
        assert!(!windows_acl_grants_untrusted_access(
            read_traverse,
            false,
            true
        ));
        assert!(!windows_acl_grants_untrusted_access(0x6, false, true));
        assert!(windows_acl_grants_untrusted_access(0x6, false, false));
        for right in [0x40, 0x10000, 0x40000, 0x80000, 0x10000000, 0x40000000] {
            assert!(windows_acl_grants_untrusted_access(right, false, true));
        }
        for root in [false, true] {
            assert!(windows_acl_grants_untrusted_access(
                read_traverse,
                true,
                root
            ));
            assert!(windows_acl_grants_untrusted_access(0x6, true, root));
            assert!(!windows_acl_grants_untrusted_access(0, true, root));
        }
    }

    #[test]
    fn var_name_rules_match_placeholder_shape() {
        assert!(valid_var_name("GROK_API_KEY"));
        assert!(valid_var_name("_x1"));
        assert!(!valid_var_name(""));
        assert!(!valid_var_name("1BAD"));
        assert!(!valid_var_name("has-dash"));
        assert!(!valid_var_name("has space"));
    }

    #[test]
    fn env_parser_supports_quotes_and_rejects_duplicates() {
        let parsed = parse_env("# comment\nA=plain\nB='json value'\nC=\"line\\nvalue\"\n").unwrap();
        assert_eq!(parsed.get("A").map(String::as_str), Some("plain"));
        assert_eq!(parsed.get("B").map(String::as_str), Some("json value"));
        assert_eq!(parsed.get("C").map(String::as_str), Some("line\nvalue"));
        assert!(parse_env("A=one\nA=two\n").is_err());
        assert!(parse_env("A=one\na=two\n").is_err());
        assert!(parse_env("bad-name=value\n").is_err());
    }

    #[test]
    fn updater_preserves_unowned_lines_and_can_remove() {
        let mut updates = BTreeMap::new();
        updates.insert("A".into(), Some("new value".into()));
        updates.insert("C".into(), Some("three".into()));
        updates.insert("B".into(), None);
        let rendered = render_updated("# keep\nA=old\nB=two\n", &updates).unwrap();
        assert!(rendered.starts_with("# keep\n"));
        assert!(rendered.contains("A=\"new value\"\n"));
        assert!(!rendered.contains("B="));
        assert!(rendered.contains("C=\"three\"\n"));
    }

    #[test]
    fn empty_file_value_is_not_reported_as_a_configured_secret() {
        CACHE.with(|cache| {
            *cache.borrow_mut() = Snapshot {
                values: BTreeMap::from([("EMPTY_TEST_SECRET".into(), String::new())]),
                ..Snapshot::default()
            };
        });
        assert_eq!(lookup("EMPTY_TEST_SECRET"), None);
        assert!(!is_set("EMPTY_TEST_SECRET"));
        assert!(!is_stored("EMPTY_TEST_SECRET"));
    }
    #[test]
    fn selected_source_error_or_missing_key_never_falls_back_to_parent_environment() {
        const NAME: &str = "AM_SELECTED_SOURCE_NO_FALLBACK";
        std::env::set_var(NAME, "parent-account-value");
        for snapshot in [
            Snapshot {
                configured_path: Some("/private-fixture/source.env".into()),
                ..Snapshot::default()
            },
            Snapshot {
                configured_path: Some("/private-fixture/source.env".into()),
                error: Some("unreadable source".into()),
                ..Snapshot::default()
            },
            Snapshot {
                error: Some("invalid configured source".into()),
                ..Snapshot::default()
            },
        ] {
            CACHE.with(|cache| *cache.borrow_mut() = snapshot);
            assert_eq!(lookup(NAME), None);
            assert!(!is_set(NAME));
            assert_eq!(std::env::var(NAME).unwrap(), "parent-account-value");
        }
        CACHE.with(|cache| *cache.borrow_mut() = Snapshot::default());
        assert_eq!(lookup(NAME).as_deref(), Some("parent-account-value"));
        assert!(ensure_source_ready().is_err());
        assert!(lookup_account(NAME).is_err());
        std::env::remove_var(NAME);
    }
}
