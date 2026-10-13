//! Extension target locks, protected identity variables and clean MCP subprocesses.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::native::Context;
use crate::native_profiles::model::{self, ProfileKind};
use crate::platform::Canon;

pub(crate) struct Account {
    pub dir: PathBuf,
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
        Ok(Account { dir })
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
            dir: crate::native_profiles::paths::validate_primary_account(ctx, kind)?,
        })
    } else {
        bail!("账号必须显式使用 agents: 或 profiles: 标识");
    }
}

pub(crate) use crate::native_account_locks::{lock_directory, AccountLock};

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
    let mut directories: std::collections::BTreeMap<String, std::collections::BTreeSet<PathBuf>> =
        std::collections::BTreeMap::new();
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
                directories
                    .entry(crate::native_account_locks::account_key(&real)?)
                    .or_default()
                    .insert(real);
            }
        }
    }
    directories
        .into_iter()
        .map(|(_, dirs)| {
            crate::native_account_locks::lock_aliases(
                ctx,
                &dirs.into_iter().collect::<Vec<_>>(),
                true,
            )
        })
        .collect()
}

fn resolve_missing(path: &Path) -> Result<PathBuf> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(path.canon()?),
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

pub(crate) fn identity_override_env(name: &str) -> bool {
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

pub fn run_cli(_args: &[String]) -> Result<i32> {
    bail!("账号与启动命令管理已移除；请直接使用助手自己的官方命令")
}
