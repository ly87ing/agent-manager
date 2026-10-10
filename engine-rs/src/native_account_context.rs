//! Explicit account contexts shared by terminal launch, authentication and probes.
//! The launcher re-reads registration on every invocation; it never contains a key.

use anyhow::{anyhow, bail, Context as _, Result};
use fs2::FileExt;
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::native::{Context, Request};
use crate::native_profiles::model::{self, AccountAuthMode, ProfileDef, ProfileKind};
use crate::platform::Canon;

pub(crate) struct Account {
    pub only: String,
    pub kind: ProfileKind,
    pub bin: String,
    pub dir: PathBuf,
    pub profile: Option<ProfileDef>,
    pub mode: AccountAuthMode,
}

pub(crate) fn resolve_account(ctx: &Context, only: &str) -> Result<Account> {
    if let Some(id) = only.strip_prefix("profiles:") {
        let policy = crate::store::effective_policy(ctx)?;
        let (_, _, parsed) = model::load_definition_entries(&policy)
            .into_iter()
            .find(|(registered, _, _)| registered == id)
            .ok_or_else(|| anyhow!("账号未登记或已删除，旧启动命令已失效"))?;
        let def = parsed?;
        if !def.enabled {
            bail!("账号已停用，拒绝启动或认证操作");
        }
        let dir = crate::native_profiles::paths::config_dir(ctx, &def)?;
        let bin = crate::native_profiles::status::base_bin(ctx, &def);
        Ok(Account {
            only: only.into(),
            kind: def.kind,
            bin,
            dir,
            mode: model::account_auth_mode(&def),
            profile: Some(def),
        })
    } else if let Some(id) = only.strip_prefix("agents:") {
        let expected_kind = match id {
            "claude-code" => ProfileKind::Claude,
            "codex" => ProfileKind::Codex,
            _ => bail!("受控主账号必须使用固定 claude-code 或 codex 标识"),
        };
        let def = crate::native_agents::load_definitions(ctx)?
            .into_iter()
            .find(|def| def.id == id)
            .ok_or_else(|| anyhow!("没有这个主账号"))?;
        if !def.enabled || def.spec_ref().get("profile").is_some() {
            bail!("主账号已停用或选择了独立环境，拒绝操作");
        }
        let kind = match crate::native_agent_adapters::kind_for_bin(&def.bin) {
            Some(crate::native_agent_adapters::AdapterKind::Claude) => ProfileKind::Claude,
            Some(crate::native_agent_adapters::AdapterKind::Codex) => ProfileKind::Codex,
            None => bail!("受控账号入口只支持 Claude Code 与 Codex"),
        };
        if kind != expected_kind {
            bail!("主账号登记的 CLI 类型与固定标识不一致，拒绝操作");
        }
        Ok(Account {
            only: only.into(),
            kind,
            bin: def.bin,
            dir: crate::native_profiles::paths::validate_primary_account(ctx, kind)?,
            profile: None,
            mode: AccountAuthMode::Official,
        })
    } else {
        bail!("账号必须显式使用 agents: 或 profiles: 标识");
    }
}

/// Held for the child lifetime. Shared launch locks allow parallel sessions;
/// exclusive authentication/configuration locks refuse changes to a running account.
pub(crate) struct AccountLock {
    _file: File,
}

pub(crate) fn lock_directory(ctx: &Context, dir: &Path, write: bool) -> Result<AccountLock> {
    let dir = resolve_missing(dir)?;
    let locks = ctx.data_dir.join("account-locks");
    std::fs::create_dir_all(&locks)?;
    let name = crate::native_mcp::model::sha256_hex(&dir.to_string_lossy());
    let path = locks.join(name);
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        if !meta.is_file() || meta.file_type().is_symlink() {
            bail!("账号锁路径不安全，拒绝操作");
        }
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)?;
    let result = if write {
        file.try_lock_exclusive()
    } else {
        FileExt::try_lock_shared(&file)
    };
    result.map_err(|_| anyhow!("该账号有正在运行的会话或认证/配置操作，拒绝并发修改"))?;
    Ok(AccountLock { _file: file })
}

pub(crate) fn lock_account_write(ctx: &Context, only: &str) -> Result<AccountLock> {
    let account = resolve_account(ctx, only)?;
    lock_directory(ctx, &account.dir, true)
}

/// Policy edits must lock the old location as well as the candidate location.
/// Read raw registrations so a malformed legacy account can still be repaired
/// or removed without losing the lock protecting an existing running location.
pub(crate) fn lock_policy_account_changes(
    ctx: &Context,
    before: &Value,
    after: &Value,
) -> Result<Vec<AccountLock>> {
    let before = before
        .pointer("/agents/definitions")
        .and_then(Value::as_object);
    let after = after
        .pointer("/agents/definitions")
        .and_then(Value::as_object);
    let ids: std::collections::BTreeSet<_> = before
        .into_iter()
        .chain(after)
        .flat_map(|defs| defs.keys())
        .collect();
    let mut directories = std::collections::BTreeSet::new();
    for id in ids {
        let old = before.and_then(|defs| defs.get(id));
        let new = after.and_then(|defs| defs.get(id));
        if old == new {
            continue;
        }
        for raw in old.into_iter().chain(new) {
            let dir = if let Some(raw) = raw.pointer("/profile/config_dir").and_then(Value::as_str)
            {
                model::expand_home(raw.trim(), &ctx.home_dir)
            } else if id == "claude-code" {
                ctx.home_dir.join(".claude")
            } else if id == "codex" {
                ctx.home_dir.join(".codex")
            } else {
                continue;
            };
            if !dir.is_absolute() || dir.components().any(|c| c.as_os_str() == "..") {
                continue;
            }
            if let Ok(real) = resolve_missing(&dir) {
                directories.insert(real);
            }
        }
    }
    directories
        .into_iter()
        .map(|dir| lock_directory(ctx, &dir, true))
        .collect()
}

fn resolve_missing(path: &Path) -> Result<PathBuf> {
    match path.canon() {
        Ok(path) => Ok(path),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| anyhow!("账号路径无法解析"))?;
            Ok(resolve_missing(parent)?.join(
                path.file_name()
                    .ok_or_else(|| anyhow!("账号路径无法解析"))?,
            ))
        }
        Err(err) => Err(err.into()),
    }
}

pub(crate) fn unsafe_account_alias(value: &str) -> bool {
    if value.contains("--account-launch") {
        return false;
    }
    let token = regex::Regex::new(
        r"(?i)(^|[^A-Za-z0-9_-])(claude|codex)([.](cmd|exe|bat))?($|[^A-Za-z0-9_-])",
    )
    .expect("fixed pattern");
    token.is_match(value) || value.contains("CLAUDE_CONFIG_DIR") || value.contains("CODEX_HOME")
}

pub(crate) fn validate_alias(ctx: &Context, value: &str) -> Result<()> {
    if unsafe_account_alias(value) {
        bail!("助手快捷命令缺少账号预检，请使用账号页生成的受控启动命令；原命令未修改");
    }
    if !value.contains("--account-launch") {
        return Ok(());
    }
    // Generated commands have one exact trusted prefix. No shell operators,
    // environment assignments or executable substitutions may precede it.
    let policy = crate::store::effective_policy(ctx)?;
    let mut ids = vec!["agents:claude-code".to_string(), "agents:codex".to_string()];
    ids.extend(
        model::load_definition_entries(&policy)
            .into_iter()
            .map(|(id, _, _)| format!("profiles:{id}")),
    );
    for only in ids {
        let Ok(expected) = launch_value(ctx, &only, "launch") else {
            continue;
        };
        if let Some(suffix) = value.strip_prefix(&expected) {
            if suffix.chars().any(|c| {
                matches!(
                    c,
                    '\'' | '"' | '$' | '`' | ';' | '&' | '|' | '<' | '>' | '(' | ')' | '\\'
                )
            }) {
                bail!("受控启动命令附加参数包含 shell 表达式，拒绝写入");
            }
            let args: Vec<_> = suffix.split_whitespace().map(str::to_string).collect();
            validate_cli_args(&args)?;
            resolve_account(ctx, &only)?;
            return Ok(());
        }
    }
    bail!("启动命令与当前受控入口不一致或账号未登记，拒绝写入")
}

/// A draft command is display-only. The runtime binding can never equal this
/// marker, and the write path continues to require a registered live binding.
pub(crate) fn validate_alias_for_preview(ctx: &Context, value: &str) -> Result<()> {
    if !value.contains("--binding preview-only") {
        return validate_alias(ctx, value);
    }
    let pattern = regex::Regex::new(
        r#" --only "profiles:([a-z0-9][a-z0-9-]*)" --operation launch --binding preview-only --"#,
    )
    .expect("fixed pattern");
    let id = pattern
        .captures(value)
        .and_then(|capture| capture.get(1))
        .ok_or_else(|| anyhow!("候选启动命令格式不正确"))?
        .as_str();
    if !model::valid_profile_id(id) {
        bail!("候选账号标识不合法");
    }
    let expected = render_launch_value(ctx, &format!("profiles:{id}"), "launch", "preview-only")?;
    let suffix = value
        .strip_prefix(&expected)
        .ok_or_else(|| anyhow!("候选启动命令与当前受控入口或账号上下文不一致"))?;
    if suffix.chars().any(|c| {
        matches!(
            c,
            '\'' | '"' | '$' | '`' | ';' | '&' | '|' | '<' | '>' | '(' | ')' | '\\'
        )
    }) {
        bail!("候选命令包含 shell 表达式");
    }
    validate_cli_args(
        &suffix
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>(),
    )
}

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

pub(crate) fn read_config(path: &Path) -> Result<Option<Vec<u8>>> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => bail!("相关配置不可读取，无法确认账号隔离"),
    };
    if !meta.is_file() || meta.len() > MAX_CONFIG_BYTES {
        bail!("相关配置不是普通文件或超过大小限制，无法确认账号隔离");
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| anyhow!("相关配置不可读取，无法确认账号隔离"))?
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow!("相关配置不可读取，无法确认账号隔离"))?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        bail!("相关配置超过大小限制");
    }
    Ok(Some(bytes))
}

fn identity_env(name: &str) -> bool {
    identity_override_env(name) || crate::native::credential_like_name(name)
}

pub(crate) fn process_override_env(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "HOME"
            | "USERPROFILE"
            | "HOMEDRIVE"
            | "HOMEPATH"
            | "XDG_CONFIG_HOME"
            | "PATH"
            | "SYSTEMROOT"
            | "WINDIR"
            | "COMSPEC"
            | "PATHEXT"
            | "APPDATA"
            | "LOCALAPPDATA"
            | "PROGRAMDATA"
            | "PROGRAMFILES"
            | "NODE_OPTIONS"
            | "LD_PRELOAD"
            | "DYLD_INSERT_LIBRARIES"
    )
}

fn identity_override_env(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    [
        "ANTHROPIC_",
        "CLAUDE_",
        "CODEX_",
        "OPENAI_",
        "AWS_",
        "AZURE_",
        "GOOGLE_",
        "GOOGLE_APPLICATION_",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
        || matches!(
            name.as_str(),
            "HOME"
                | "USERPROFILE"
                | "HOMEDRIVE"
                | "HOMEPATH"
                | "XDG_CONFIG_HOME"
                | "PATH"
                | "SYSTEMROOT"
                | "WINDIR"
                | "COMSPEC"
                | "PATHEXT"
                | "APPDATA"
                | "LOCALAPPDATA"
                | "PROGRAMDATA"
                | "PROGRAMFILES"
                | "NODE_OPTIONS"
                | "LD_PRELOAD"
                | "DYLD_INSERT_LIBRARIES"
        )
}

fn json_hook_has_content(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.values().any(json_hook_has_content),
        Value::Array(items) => items.iter().any(json_hook_has_content),
        _ => true,
    }
}

fn json_has_active_hooks(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            (key == "hooks" && json_hook_has_content(value)) || json_has_active_hooks(value)
        }),
        Value::Array(items) => items.iter().any(json_has_active_hooks),
        _ => false,
    }
}

fn json_has_identity(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            matches!(
                key.as_str(),
                "apiKeyHelper"
                    | "policyHelper"
                    | "forceLoginMethod"
                    | "forceLoginOrgUUID"
                    | "forceLoginGatewayUrl"
                    | "gatewayInternalNetworks"
                    | "otelHeadersHelper"
            ) || (key == "env"
                && value
                    .as_object()
                    .map_or(true, |env| env.keys().any(|key| identity_env(key))))
                || (key == "hooks" && json_hook_has_content(value))
                || json_has_identity(value)
        }),
        Value::Array(items) => items.iter().any(json_has_identity),
        _ => false,
    }
}

fn check_json_layer(path: &Path) -> Result<()> {
    if let Some(bytes) = read_config(path)? {
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow!("相关 settings 配置损坏，无法确认账号隔离"))?;
        if !value.is_object() || json_has_identity(&value) {
            bail!("项目或托管 settings 包含账号、凭据或 endpoint 覆盖，拒绝操作");
        }
    }
    Ok(())
}

fn toml_hook_has_content(value: &toml::Value) -> bool {
    match value {
        toml::Value::Table(table) => table.values().any(toml_hook_has_content),
        toml::Value::Array(items) => items.iter().any(toml_hook_has_content),
        _ => true,
    }
}

fn toml_has_active_hooks(table: &toml::Table) -> bool {
    table.iter().any(|(key, value)| {
        (matches!(key.as_str(), "hooks" | "notify" | "experimental_hooks")
            && toml_hook_has_content(value))
            || value.as_table().is_some_and(toml_has_active_hooks)
            || value.as_array().is_some_and(|items| {
                items
                    .iter()
                    .any(|value| value.as_table().is_some_and(toml_has_active_hooks))
            })
    })
}

fn toml_has_identity(table: &toml::Table) -> bool {
    table.iter().any(|(key, value)| {
        matches!(
            key.as_str(),
            "model_provider"
                | "model_providers"
                | "profile"
                | "profiles"
                | "openai_base_url"
                | "chatgpt_base_url"
                | "forced_login_method"
                | "forced_chatgpt_workspace_id"
                | "allowed_login_methods"
                | "allowed_chatgpt_workspaces"
                | "cli_auth_credentials_store"
                | "experimental_bearer_token"
                | "env_key"
                | "env_http_headers"
                | "http_headers"
                | "api_key"
                | "auth"
                | "auth_mode"
        ) || (matches!(key.as_str(), "hooks" | "notify" | "experimental_hooks")
            && toml_hook_has_content(value))
            || value.as_table().is_some_and(toml_has_identity)
    })
}

fn check_toml_layer(path: &Path) -> Result<()> {
    if let Some(bytes) = read_config(path)? {
        let text = std::str::from_utf8(&bytes).map_err(|_| anyhow!("相关 TOML 配置不是 UTF-8"))?;
        let table: toml::Table = text
            .parse()
            .map_err(|_| anyhow!("相关 TOML 配置损坏，无法确认账号隔离"))?;
        if toml_has_identity(&table) {
            bail!("项目或系统 TOML 包含账号、凭据或 endpoint 覆盖，拒绝操作");
        }
    }
    Ok(())
}

fn system_dir(kind: ProfileKind) -> Result<PathBuf> {
    Ok(match kind {
        ProfileKind::Claude => {
            #[cfg(target_os = "macos")]
            {
                PathBuf::from("/Library/Application Support/ClaudeCode")
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                PathBuf::from("/etc/claude-code")
            }
            #[cfg(windows)]
            {
                windows_system_dirs()?.2.join("ClaudeCode")
            }
        }
        ProfileKind::Codex => {
            #[cfg(unix)]
            {
                PathBuf::from("/etc/codex")
            }
            #[cfg(windows)]
            {
                windows_system_dirs()?.1.join("OpenAI").join("Codex")
            }
        }
    })
}

#[cfg(windows)]
fn windows_system_dirs() -> Result<(PathBuf, PathBuf, PathBuf)> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    let registry = winreg::RegKey::predef(HKEY_LOCAL_MACHINE);
    let read = |key: &str, name: &str| -> Result<PathBuf> {
        let raw: String = registry
            .open_subkey(key)
            .and_then(|key| key.get_value(name))
            .map_err(|_| anyhow!("无法读取可信系统目录，拒绝账号操作"))?;
        let path = PathBuf::from(raw);
        if !path.is_absolute() || path.components().any(|part| part.as_os_str() == "..") {
            bail!("系统目录无法可靠解析，拒绝账号操作");
        }
        Ok(path)
    };
    Ok((
        read(
            "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
            "SystemRoot",
        )?,
        read(
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Shell Folders",
            "Common AppData",
        )?,
        read(
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion",
            "ProgramFilesDir",
        )?,
    ))
}

pub(crate) fn validate_layers(account: &Account, cwd: &Path) -> Result<()> {
    // Inspect every ancestor: the CLI can discover project roots above cwd.
    for dir in cwd.ancestors() {
        match account.kind {
            ProfileKind::Claude => {
                check_json_layer(&dir.join(".claude/settings.json"))?;
                check_json_layer(&dir.join(".claude/settings.local.json"))?;
            }
            ProfileKind::Codex => check_toml_layer(&dir.join(".codex/config.toml"))?,
        }
    }
    validate_managed(account, &system_dir(account.kind)?)?;
    if account.kind == ProfileKind::Codex {
        check_toml_layer(&cwd.join("config.toml"))?;
    }
    if account.kind == ProfileKind::Claude {
        if let Some(bytes) = read_config(&account.dir.join("settings.json"))? {
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|_| anyhow!("账号 settings 配置损坏"))?;
            if json_has_active_hooks(&value) {
                bail!("账号配置存在可执行 hooks，无法证明其不会改写身份或 endpoint，拒绝操作");
            }
        }
        check_json_layer(&account.dir.join("settings.local.json"))?;
        for name in ["managed-settings.json", "managed-settings-cache.json"] {
            check_json_layer(&account.dir.join(name))?;
        }
    }
    if account.kind == ProfileKind::Codex {
        if let Some(bytes) = read_config(&account.dir.join("config.toml"))? {
            let text = std::str::from_utf8(&bytes).map_err(|_| anyhow!("账号 TOML 不是 UTF-8"))?;
            let table: toml::Table = text.parse().map_err(|_| anyhow!("账号 TOML 配置损坏"))?;
            if toml_has_active_hooks(&table) {
                bail!(
                    "账号配置存在可执行 hook/notify，无法证明其不会改写身份或 endpoint，拒绝操作"
                );
            }
        }
    }
    validate_os_policy(account.kind)?;
    Ok(())
}

fn validate_managed(account: &Account, dir: &Path) -> Result<()> {
    match account.kind {
        ProfileKind::Claude => {
            check_json_layer(&dir.join("managed-settings.json"))?;
            match std::fs::read_dir(dir.join("managed-settings.d")) {
                Ok(entries) => {
                    for entry in entries {
                        let path = entry.map_err(|_| anyhow!("托管配置目录不可读取"))?.path();
                        if path.extension().is_some_and(|ext| ext == "json") {
                            check_json_layer(&path)?;
                        }
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => bail!("托管配置目录不可读取，无法确认账号隔离"),
            }
        }
        ProfileKind::Codex => {
            check_toml_layer(&dir.join("config.toml"))?;
            check_toml_layer(&dir.join("requirements.toml"))?;
            check_toml_layer(&dir.join("managed_config.toml"))?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn validate_os_policy(kind: ProfileKind) -> Result<()> {
    use std::ffi::{c_char, c_void, CString};
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            text: *const c_char,
            encoding: u32,
        ) -> *const c_void;
        fn CFPreferencesAppValueIsForced(key: *const c_void, application_id: *const c_void) -> u8;
        fn CFRelease(value: *const c_void);
    }
    struct CFString(*const c_void);
    impl CFString {
        fn new(text: &str) -> Result<Self> {
            let text = CString::new(text)?;
            // UTF-8, copied by CF; the CString need not outlive this call.
            let ptr =
                unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), 0x08000100) };
            if ptr.is_null() {
                bail!("无法检查系统托管偏好设置");
            }
            Ok(Self(ptr))
        }
    }
    impl Drop for CFString {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) };
        }
    }
    let (domain, keys): (&str, &[&str]) = match kind {
        ProfileKind::Claude => (
            "com.anthropic.claudecode",
            &[
                "env",
                "apiKeyHelper",
                "policyHelper",
                "forceLoginMethod",
                "forceLoginOrgUUID",
                "forceLoginGatewayUrl",
                "gatewayInternalNetworks",
                "managedSettings",
                "settings",
                "hooks",
            ],
        ),
        ProfileKind::Codex => (
            "com.openai.codex",
            &["config_toml_base64", "requirements_toml_base64"],
        ),
    };
    let domain = CFString::new(domain)?;
    for key in keys {
        let key = CFString::new(key)?;
        if unsafe { CFPreferencesAppValueIsForced(key.0, domain.0) } != 0 {
            bail!("存在影响账号或 endpoint 的系统托管偏好设置，无法可靠绑定账号上下文，拒绝操作");
        }
    }
    Ok(())
}

#[cfg(windows)]
fn validate_os_policy(kind: ProfileKind) -> Result<()> {
    if kind != ProfileKind::Claude {
        return Ok(());
    }
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        match winreg::RegKey::predef(root).open_subkey("SOFTWARE\\Policies\\ClaudeCode") {
            Ok(key) => {
                let value = key.get_value::<String, _>("Settings");
                match value {
                    Ok(text) => {
                        if text.len() as u64 > MAX_CONFIG_BYTES {
                            bail!("托管注册表设置过大");
                        }
                        let value: Value = serde_json::from_str(&text)
                            .map_err(|_| anyhow!("托管注册表设置损坏"))?;
                        if !value.is_object() || json_has_identity(&value) {
                            bail!("托管注册表设置包含账号或 endpoint 覆盖");
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => bail!("托管注册表设置不可读取"),
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("托管注册表设置不可读取"),
        }
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn validate_os_policy(_: ProfileKind) -> Result<()> {
    Ok(())
}

pub(crate) fn validate_auth(ctx: &Context, account: &Account) -> Result<()> {
    model::validate_account_auth(
        ctx,
        account
            .profile
            .as_ref()
            .map_or(account.only.as_str(), |def| def.id.as_str()),
        account.kind,
        account.profile.as_ref().map(|_| account.dir.as_path()),
        account.mode,
    )
}

fn resolve_cli(ctx: &Context, account: &Account) -> Result<PathBuf> {
    let bin = Path::new(&account.bin);
    let candidate = if bin.is_absolute() {
        bin.to_path_buf()
    } else {
        if bin.components().count() != 1 {
            bail!("CLI 必须使用命令名或绝对路径");
        }
        let dirs = std::iter::once(ctx.home_dir.join(".local/bin"))
            .chain(
                std::env::var_os("PATH")
                    .map(|paths| {
                        std::env::split_paths(&paths)
                            .filter(|p| p.is_absolute())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            )
            .chain(crate::native_agents::fallback_bin_dirs(&ctx.home_dir));
        crate::platform::find_command(dirs, &account.bin)
            .ok_or_else(|| anyhow!("找不到可执行的助手 CLI"))?
    };
    if !crate::platform::is_executable(&candidate) {
        bail!("CLI 不是可执行的普通文件");
    }
    #[cfg(windows)]
    if !candidate
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        bail!("受控账号需要原生 CLI .exe；无法可靠净化 .cmd/.bat/npm 包装命令，请安装受支持版本的原生 CLI 后重新生成命令");
    }
    candidate.canon().map_err(Into::into)
}

/// A short-lived directory prevents authentication commands from discovering
/// the caller's project configuration. Account and system guards still apply.
pub(crate) struct NeutralWorkingDir(PathBuf);

impl NeutralWorkingDir {
    pub(crate) fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "am-account-probe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::create_dir(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self(path.canon()?))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for NeutralWorkingDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Credential precedence and config layering are an explicit compatibility
/// contract, rather than a promise about arbitrary future CLI releases.
pub(crate) fn validate_cli_version(ctx: &Context, account: &Account) -> Result<()> {
    let cli = resolve_cli(ctx, account)?;
    let root = std::env::temp_dir().join(format!(
        "am-account-version-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    std::fs::create_dir(&root)?;
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let version_ctx = Context {
        home_dir: root.clone(),
        data_dir: root.join("data"),
    };
    let version_account = Account {
        only: "version-probe".into(),
        kind: account.kind,
        bin: cli.to_string_lossy().into(),
        dir: root.join("config"),
        profile: None,
        mode: AccountAuthMode::Official,
    };
    let mut command = command_for(&version_ctx, &version_account)?;
    command.arg("--version").current_dir(&root);
    let (code, stdout, _) =
        crate::native_upstream::capture_exit(command, std::time::Duration::from_secs(10))
            .ok_or_else(|| anyhow!("CLI 版本检查超时或失败，拒绝账号操作"))?;
    let text = std::str::from_utf8(&stdout)
        .map_err(|_| anyhow!("CLI 版本输出无法识别"))?
        .trim();
    let supported = match account.kind {
        ProfileKind::Claude => text == "2.1.285 (Claude Code)",
        ProfileKind::Codex => matches!(
            text,
            "codex-cli 0.156.1" | "codex-cli 0.159.0" | "codex-cli 0.162.1"
        ),
    };
    if code != 0 || !supported {
        bail!("CLI 版本未纳入受控认证契约，拒绝操作；当前支持 Claude Code 2.1.285、Codex CLI 0.156.1/0.159.0/0.162.1");
    }
    Ok(())
}

const SAFE_ENV: &[&str] = &[
    "TERM",
    "COLORTERM",
    "TERM_PROGRAM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    "TMPDIR",
    "TMP",
    "TEMP",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NODE_EXTRA_CA_CERTS",
];

pub(crate) fn clean_command(ctx: &Context, program: &Path) -> Result<Command> {
    if !program.is_absolute() {
        bail!("受控子进程需要已解析的绝对路径");
    }
    let mut command = crate::process::command(program);
    command.env_clear();
    for name in SAFE_ENV {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let dirs = std::iter::once(
        program
            .parent()
            .ok_or_else(|| anyhow!("子进程路径没有父目录"))?
            .to_path_buf(),
    )
    .chain(
        std::env::var_os("PATH")
            .map(|paths| {
                std::env::split_paths(&paths)
                    .filter(|p| p.is_absolute())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    )
    .chain(crate::native_agents::fallback_bin_dirs(&ctx.home_dir));
    command
        .env("PATH", std::env::join_paths(dirs)?)
        .env("HOME", &ctx.home_dir)
        .env("USERPROFILE", &ctx.home_dir);
    #[cfg(windows)]
    {
        let (windows, program_data, program_files) = windows_system_dirs()?;
        command
            .env("SystemRoot", &windows)
            .env("WINDIR", &windows)
            .env("COMSPEC", windows.join("System32").join("cmd.exe"))
            .env("PATHEXT", ".EXE;.COM;.BAT;.CMD")
            .env("APPDATA", ctx.home_dir.join("AppData").join("Roaming"))
            .env("LOCALAPPDATA", ctx.home_dir.join("AppData").join("Local"))
            .env("ProgramData", program_data)
            .env("ProgramFiles", program_files);
    }
    Ok(command)
}

pub(crate) fn command_for(ctx: &Context, account: &Account) -> Result<Command> {
    let cli = resolve_cli(ctx, account)?;
    let mut command = clean_command(ctx, &cli)?;
    // Global Anthropic profiles/WIF are outside a Claude configuration directory.
    // A fixed nonexistent path prevents a global profile from authenticating it.
    let no_profile = account.dir.join(".agent-manager-no-anthropic-profile");
    match std::fs::symlink_metadata(&no_profile) {
        Ok(_) => bail!("账号的全局认证屏蔽路径已被占用，拒绝启动"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => bail!("账号的全局认证屏蔽路径不可检查"),
    }
    command.env("ANTHROPIC_CONFIG_DIR", no_profile);
    if account.profile.is_some() || account.kind == ProfileKind::Codex {
        command.env(account.kind.config_env(), &account.dir);
    }
    if let Some(def) = &account.profile {
        for (name, value) in &def.env {
            if process_override_env(name) {
                bail!("账号定义会改写受控运行时目录或进程上下文，拒绝启动");
            }
            if matches!(
                name.as_str(),
                "HOME"
                    | "USERPROFILE"
                    | "CLAUDE_CONFIG_DIR"
                    | "CODEX_HOME"
                    | "ANTHROPIC_CONFIG_DIR"
                    | "PATH"
                    | "XDG_CONFIG_HOME"
                    | "NODE_OPTIONS"
                    | "LD_PRELOAD"
                    | "DYLD_INSERT_LIBRARIES"
            ) {
                bail!("账号定义包含进程或配置目录覆盖，拒绝启动");
            }
            let text = value
                .as_str()
                .ok_or_else(|| anyhow!("账号环境变量类型错误"))?;
            let resolved = match text.strip_prefix("${").and_then(|v| v.strip_suffix('}')) {
                Some(name) => crate::secret_values::lookup_account(name)
                    .context("账号凭据来源不可用，拒绝启动")?,
                None => text.to_string(),
            };
            command.env(name, resolved);
        }
    }
    if account.only.starts_with("agents:") || account.only.starts_with("profiles:") {
        for name in crate::native_secrets::account_mcp_secret_names(ctx, &account.only)? {
            if identity_override_env(&name) {
                bail!("MCP 凭据引用会改写助手身份或进程上下文，拒绝启动");
            }
            let value = crate::secret_values::lookup_account(&name)
                .context("账号 MCP 凭据来源缺少引用，拒绝启动")?;
            command.env(name, value);
        }
    }
    Ok(command)
}

fn validate_cli_args(args: &[String]) -> Result<()> {
    // Unknown options fail closed. Ordinary prompts, resume IDs and the supported
    // session/model/permission switches cannot change authentication or routing.
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg.chars().any(|c| c == '\0' || c == '\r' || c == '\n') {
            bail!("CLI 参数含非法控制字符");
        }
        if arg.starts_with('-') {
            let flag = arg.split('=').next().unwrap_or(arg);
            let takes_value = matches!(
                flag,
                "--model"
                    | "-m"
                    | "--resume"
                    | "-r"
                    | "--session-id"
                    | "--permission-mode"
                    | "--output-format"
                    | "--input-format"
                    | "--max-turns"
                    | "--max-budget-usd"
                    | "--effort"
            );
            let boolean = matches!(
                flag,
                "--dangerously-skip-permissions"
                    | "--allow-dangerously-skip-permissions"
                    | "--full-auto"
                    | "--dangerously-bypass-approvals-and-sandbox"
                    | "--continue"
                    | "--fork-session"
                    | "--print"
                    | "-p"
                    | "--verbose"
                    | "--help"
                    | "-h"
                    | "--version"
                    | "-V"
            );
            if !takes_value && !boolean {
                bail!("CLI 参数不在受控允许列表，可能覆盖账号或 endpoint，拒绝启动");
            }
            if takes_value && !arg.contains('=') {
                index += 1;
                if args.get(index).is_none_or(|value| value.starts_with('-')) {
                    bail!("CLI 参数缺少取值");
                }
            }
        } else if matches!(
            arg.as_str(),
            "login" | "logout" | "auth" | "app-server" | "mcp" | "serve" | "proxy" | "completion"
        ) {
            bail!("认证或服务子命令必须经显式受控操作执行");
        }
        index += 1;
    }
    Ok(())
}

/// Native desktop and engine CLI call this before their ordinary startup paths.
/// `args` excludes the leading `--account-launch` marker. No JSON envelope is
/// emitted into the assistant's interactive terminal stream.
pub fn run_cli(args: &[String]) -> Result<i32> {
    prepare_terminal_console()?;
    let mut home = None;
    let mut data = None;
    let mut only = None;
    let mut operation = None;
    let mut binding = None;
    let mut index = 0;
    while index < args.len() && args[index] != "--" {
        let slot = match args[index].as_str() {
            "--home" => &mut home,
            "--data-dir" => &mut data,
            "--only" => &mut only,
            "--operation" => &mut operation,
            "--binding" => &mut binding,
            _ => bail!("受控入口存在未知参数"),
        };
        if slot.is_some() {
            bail!("受控入口参数重复");
        }
        index += 1;
        let value = args
            .get(index)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| anyhow!("受控入口参数缺少取值"))?;
        *slot = Some(value.clone());
        index += 1;
    }
    if args.get(index).map(String::as_str) != Some("--") {
        bail!("受控入口需要显式参数分隔符");
    }
    let home = PathBuf::from(home.ok_or_else(|| anyhow!("缺少可信 HOME"))?);
    let data = PathBuf::from(data.ok_or_else(|| anyhow!("缺少数据目录"))?);
    if !home.is_absolute() || !data.is_absolute() {
        bail!("受控入口 HOME 与数据目录必须是绝对路径");
    }
    let ctx = Context {
        home_dir: home.canon()?,
        data_dir: data,
    };
    let only = only.ok_or_else(|| anyhow!("缺少账号"))?;
    let operation = operation.ok_or_else(|| anyhow!("缺少操作"))?;
    let binding = binding.ok_or_else(|| anyhow!("启动命令没有账号代际绑定，请重新生成命令"))?;
    let cli_args = &args[index + 1..];
    validate_cli_args(cli_args)?;
    if operation != "launch" && !cli_args.is_empty() {
        bail!("认证操作不接受附加 CLI 参数");
    }
    if !matches!(operation.as_str(), "launch" | "login" | "logout" | "status") {
        bail!("未知账号操作");
    }
    crate::secret_values::refresh(&ctx);
    let before = resolve_account(&ctx, &only)?;
    if account_binding(&ctx, &before)? != binding {
        bail!("启动命令的账号身份绑定已失效，请重新生成命令");
    }
    let _lock = lock_directory(
        &ctx,
        &before.dir,
        matches!(operation.as_str(), "login" | "logout"),
    )?;
    let account = resolve_account(&ctx, &only)?;
    if before.dir != account.dir || account_binding(&ctx, &account)? != binding {
        bail!("账号登记在预检期间已改变，请重试");
    }
    validate_auth(&ctx, &account)?;
    if matches!(operation.as_str(), "login" | "logout") && account.mode != AccountAuthMode::Official
    {
        bail!("API 或显式令牌账号不能执行官方登录/退出");
    }
    let neutral_cwd = if operation == "launch" {
        None
    } else {
        Some(NeutralWorkingDir::new()?)
    };
    let cwd = match &neutral_cwd {
        Some(dir) => dir.path().to_path_buf(),
        None => std::env::current_dir()?.canon()?,
    };
    validate_layers(&account, &cwd)?;
    validate_cli_version(&ctx, &account)?;
    if account.kind == ProfileKind::Claude && account.mode != AccountAuthMode::Official {
        match crate::native_auth::official_login(
            &ctx.home_dir,
            crate::native_auth::AuthKind::Claude,
            &account.bin,
            &account.dir,
        ) {
            Ok(false) => {}
            Ok(true) => bail!("账号的凭据存储中存在另一个官方登录，拒绝 API 或显式令牌启动"),
            Err(_) => bail!("无法确认账号凭据存储没有另一份官方登录，拒绝启动"),
        }
    }
    let mut command = command_for(&ctx, &account)?;
    command.current_dir(cwd);
    match (account.kind, operation.as_str()) {
        (_, "launch") => {
            command.args(cli_args);
        }
        (ProfileKind::Claude, action) => {
            command.args(["auth", action]);
        }
        (ProfileKind::Codex, "status") => {
            command.args(["login", "status"]);
        }
        (ProfileKind::Codex, action) => {
            command.arg(action);
        }
    }
    let status = command.status().context("受控账号 CLI 未正常启动")?;
    Ok(status.code().unwrap_or(1))
}

/// A Windows GUI host must attach to the invoking terminal before it starts an
/// interactive assistant. Preserve inherited pipes/files when attaching replaces
/// the default standard handles. This changes only this process's console state.
pub fn prepare_terminal_console() -> Result<()> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{
            GetLastError, ERROR_ACCESS_DENIED, INVALID_HANDLE_VALUE,
        };
        use windows_sys::Win32::Storage::FileSystem::{GetFileType, FILE_TYPE_UNKNOWN};
        use windows_sys::Win32::System::Console::{
            AttachConsole, GetConsoleWindow, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS,
            STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        };
        if !GetConsoleWindow().is_null() {
            return Ok(());
        }
        let inherited: Vec<_> = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
            .into_iter()
            .filter_map(|kind| {
                let handle = GetStdHandle(kind);
                (!handle.is_null()
                    && handle != INVALID_HANDLE_VALUE
                    && GetFileType(handle) != FILE_TYPE_UNKNOWN)
                    .then_some((kind, handle))
            })
            .collect();
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            if GetLastError() == ERROR_ACCESS_DENIED || inherited.len() == 3 {
                return Ok(());
            }
            bail!("无法连接调用终端，请从命令提示符或 PowerShell 执行受控账号命令");
        }
        for (kind, handle) in inherited {
            if SetStdHandle(kind, handle) == 0 {
                bail!("无法保留终端重定向句柄，拒绝启动助手");
            }
        }
    }
    Ok(())
}

fn quote(text: &str) -> Result<String> {
    if text.contains('!') {
        bail!("启动路径包含 shell 历史或延迟变量展开字符，无法安全生成命令");
    }
    if text.chars().any(char::is_control) || text.contains(['\'', '"', '$', '`', '%']) {
        bail!("启动路径含不支持的 shell 字符，无法安全生成命令");
    }
    #[cfg(unix)]
    let text = text.replace('\\', "\\\\");
    Ok(format!("\"{text}\""))
}

pub(crate) fn launch_value(ctx: &Context, only: &str, operation: &str) -> Result<String> {
    if !matches!(operation, "launch" | "login" | "logout" | "status") {
        bail!("未知账号操作");
    }
    let account = resolve_account(ctx, only)?;
    render_launch_value(ctx, only, operation, &account_binding(ctx, &account)?)
}

fn render_launch_value(
    ctx: &Context,
    only: &str,
    operation: &str,
    binding: &str,
) -> Result<String> {
    Ok(format!(
        "{} --account-launch --home {} --data-dir {} --only {} --operation {} --binding {} --",
        quote(&launcher_host()?.to_string_lossy())?,
        quote(&ctx.home_dir.canon()?.to_string_lossy())?,
        quote(&ctx.data_dir.to_string_lossy())?,
        quote(only)?,
        operation,
        binding
    ))
}

fn launcher_host() -> Result<PathBuf> {
    let executable = std::env::current_exe()?.canon()?;
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(executable.as_os_str().as_bytes())?;
        let mut metadata = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // Read mount metadata; this does not modify system configuration.
        if unsafe { libc::statfs(path.as_ptr(), metadata.as_mut_ptr()) } != 0 {
            bail!("无法确认应用安装路径的挂载状态，拒绝生成启动命令");
        }
        let readonly = unsafe { metadata.assume_init() }.f_flags & (libc::MNT_RDONLY as u32) != 0;
        validate_installed_host(&executable, readonly)?;
    }
    #[cfg(target_os = "linux")]
    if executable
        .components()
        .any(|part| part.as_os_str().to_string_lossy().starts_with(".mount_"))
    {
        let appdir = std::env::var_os("APPDIR")
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("AppImage 缺少可信安装上下文，拒绝生成临时路径命令"))?
            .canon()?;
        let image = std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("AppImage 缺少持久安装文件路径"))?;
        if !appdir
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(".mount_"))
            || !executable.starts_with(&appdir)
            || !image.is_absolute()
            || !crate::platform::is_executable(&image)
        {
            bail!("AppImage 安装上下文与运行程序不一致，拒绝生成启动命令");
        }
        let image = image.canon()?;
        if image.starts_with(&appdir)
            || image
                .components()
                .any(|part| part.as_os_str().to_string_lossy().starts_with(".mount_"))
        {
            bail!("AppImage 文件没有持久安装路径，拒绝生成启动命令");
        }
        return Ok(image);
    }
    Ok(executable)
}

#[cfg(any(target_os = "macos", test))]
fn validate_installed_host(executable: &Path, readonly: bool) -> Result<()> {
    if readonly
        || executable
            .components()
            .any(|part| part.as_os_str() == "AppTranslocation")
    {
        bail!("应用正从只读磁盘映像或临时隔离路径运行，请移到 Applications 安装目录再生成启动命令");
    }
    Ok(())
}

fn account_binding(ctx: &Context, account: &Account) -> Result<String> {
    let generation = if let Some(def) = &account.profile {
        let policy = crate::store::effective_policy(ctx)?;
        let raw = policy
            .pointer("/agents/definitions")
            .and_then(Value::as_object)
            .and_then(|defs| defs.get(&def.id));
        let generation = raw.and_then(|raw| raw.get("account_generation")).and_then(Value::as_str)
            .ok_or_else(|| anyhow!("既有账号缺少安全登记代际，请显式重新保存账号定义后生成命令；没有迁移或覆盖凭据"))?;
        if generation.len() != 36
            || !generation.chars().enumerate().all(|(i, c)| {
                if [8, 13, 18, 23].contains(&i) {
                    c == '-'
                } else {
                    c.is_ascii_hexdigit()
                }
            })
        {
            bail!("账号登记代际无效，请显式重新保存账号定义");
        }
        generation.to_string()
    } else {
        "reserved-primary-v1".into()
    };
    let env: std::collections::BTreeMap<String, Value> = account
        .profile
        .as_ref()
        .map(|def| {
            def.env
                .iter()
                .filter(|(key, _)| identity_env(key))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default();
    let identity = json!({"version":1,"generation":generation,"only":account.only,"home":ctx.home_dir.canon()?,
        "directory":account.dir,"kind":account.kind.as_str(),"mode":format!("{:?}",account.mode),"env":env,
        "base_url":account.profile.as_ref().and_then(|def| def.base_url.as_ref()),
        "env_key":account.profile.as_ref().and_then(|def| def.env_key.as_ref()),
        "wire_api":account.profile.as_ref().and_then(|def| def.wire_api.as_ref())});
    Ok(crate::native_mcp::model::sha256_hex(
        &serde_json::to_string(&identity)?,
    ))
}

pub(crate) fn command_request(ctx: &Context, req: &Request) -> Result<Value> {
    let mut only = None;
    let mut operation = None;
    let mut shell = None;
    let mut definition = None;
    let mut args = req.args.iter();
    while let Some(flag) = args.next() {
        let slot = match flag.as_str() {
            "--only" => &mut only,
            "--operation" => &mut operation,
            "--shell" => &mut shell,
            "--definition" => &mut definition,
            _ => bail!("accounts:command 存在不支持参数"),
        };
        if slot.is_some() {
            bail!("accounts:command 参数重复");
        }
        *slot = Some(
            args.next()
                .ok_or_else(|| anyhow!("accounts:command 参数缺少取值"))?
                .as_str(),
        );
    }
    let only = only.ok_or_else(|| anyhow!("accounts:command 需要 --only"))?;
    let operation = operation.unwrap_or("launch");
    let value = if let Some(raw) = definition {
        if operation != "launch" {
            bail!("候选账号定义只能生成启动预览，不能生成认证操作");
        }
        let id = only
            .strip_prefix("profiles:")
            .ok_or_else(|| anyhow!("候选定义只支持独立账号"))?;
        let raw: Value = serde_json::from_str(raw).map_err(|_| anyhow!("候选定义不是合法 JSON"))?;
        let def = model::parse_definition(id, &raw, true)?;
        crate::native_profiles::paths::config_dir(ctx, &def)?;
        render_launch_value(ctx, only, operation, "preview-only")?
    } else {
        let account = resolve_account(ctx, only)?;
        validate_auth(ctx, &account)?;
        if matches!(operation, "login" | "logout") && account.mode != AccountAuthMode::Official {
            bail!("API 或显式令牌账号不能执行官方登录/退出");
        }
        launch_value(ctx, only, operation)?
    };
    let terminal = match shell.unwrap_or("posix") {
        "posix" => value.clone(),
        "powershell" => format!("& {value}"),
        _ => bail!("不支持的终端类型"),
    };
    Ok(
        json!({"element": only, "launch_value": value, "terminal_command": terminal, "controlled": true, "scope": "account-preflight-and-child-process"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).into()).collect()
    }

    #[test]
    fn allowed_session_arguments_and_identity_override_rejection() {
        assert!(validate_cli_args(&args(&[
            "--resume",
            "session-id",
            "--model",
            "test-model",
            "--print",
            "hello"
        ]))
        .is_ok());
        for values in [
            vec!["-c", "env_key=other"],
            vec!["--settings={}"],
            vec!["--profile", "other"],
            vec!["-pother"],
            vec!["--config-dir", "/other"],
            vec!["--home", "/other"],
            vec!["--endpoint", "https://other.invalid"],
            vec!["--env", "TOKEN=other"],
            vec!["login"],
            vec!["auth", "logout"],
        ] {
            assert!(validate_cli_args(&args(&values)).is_err());
        }
    }

    #[test]
    fn generated_quote_accepts_spaces_and_rejects_shell_substitution() {
        assert_eq!(
            quote("/safe path/manager").unwrap(),
            "\"/safe path/manager\""
        );
        for value in [
            "/bad'path",
            "/bad\"path",
            "/bad$path",
            "/bad`path",
            "/bad%path",
            "/bad\npath",
        ] {
            assert!(quote(value).is_err());
        }
    }

    #[test]
    fn installation_guard_accepts_writable_volumes_and_refuses_images_or_translocation() {
        assert!(validate_installed_host(
            Path::new("/Volumes/External/Manager.app/Contents/MacOS/manager"),
            false
        )
        .is_ok());
        assert!(validate_installed_host(
            Path::new("/Volumes/Installer/Manager.app/Contents/MacOS/manager"),
            true
        )
        .is_err());
        assert!(validate_installed_host(
            Path::new("/private/var/AppTranslocation/random/Manager.app"),
            false
        )
        .is_err());
    }

    #[test]
    fn strict_layer_guards_do_not_reject_ordinary_permissions_and_models() {
        assert!(!json_has_identity(
            &json!({"permissions":{"allow":["Read"]},"model":"test"})
        ));
        assert!(json_has_identity(
            &json!({"env":{"CLAUDE_CONFIG_DIR":"other"}})
        ));
        assert!(json_has_identity(&json!({"policyHelper":"external"})));
        assert!(!json_has_identity(
            &json!({"hooks":{"SessionStart":[{}, []]}})
        ));
        assert!(json_has_identity(
            &json!({"hooks":{"SessionStart":[{"command":"run"}]}})
        ));
        assert!(!toml_has_identity(
            &"model = 'test'\n[permissions]\nmode='read-only'"
                .parse()
                .unwrap()
        ));
        assert!(toml_has_identity(
            &"[profiles.alt]\nmodel_provider='other'".parse().unwrap()
        ));
        assert!(!toml_has_identity(&"notify=[]\n[hooks]".parse().unwrap()));
        assert!(toml_has_identity(&"notify=['run']".parse().unwrap()));
    }
}
