//! 在用户的 PATH 上找命令，并按真实路径判断它归谁管、是什么版本。
//!
//! 桌面应用从 Dock 启动时拿到的 PATH 不是用户的（`launchctl getenv PATH` 为空，2026-09-30 实测），
//! 所以用户 PATH 用用户自己的 shell 取：`-c`（脚本）、`-lc`（登录，Codex 执行命令的方式）、`-lic`
//! （终端新窗口）三种形态各取一次，三种都有的目录才算「稳定」，可做入口目录候选。
//!
//! 只读：不执行被检测的程序，版本从包的元数据读；只启动用户 shell 打印 PATH（不带终端、
//! stdin 为空、有超时）。

use crate::platform::Canon;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
#[cfg(not(windows))]
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::native::Context;

#[cfg(not(windows))]
const PATH_MARKER: &str = "__AGENT_MANAGER_PATH__";
#[cfg(not(windows))]
const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
const CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Default)]
pub struct UserPath {
    /// 三种形态合并后的目录，按终端新窗口（`-lic`）的顺序在前。
    pub dirs: Vec<PathBuf>,
    /// 三种形态都有的目录。
    pub stable: Vec<PathBuf>,
    /// 取不到时的原因（此时退回本进程的 PATH）。
    pub error: Option<String>,
}

#[cfg(not(windows))]
fn shell_path(shell: &str, flag: &str, home: &Path) -> Result<Vec<PathBuf>, String> {
    let mut command = Command::new(shell);
    command
        .arg(flag)
        .arg(format!("printf '\\n{PATH_MARKER}%s\\n' \"$PATH\""))
        .env("HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = crate::process::capture_group(&mut command, PROBE_TIMEOUT, 256 * 1024).map_err(
        |error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                format!("{shell} {flag} 超时")
            } else {
                format!("启动 {shell} 失败：{error}")
            }
        },
    )?;
    let out = String::from_utf8(output.stdout).unwrap_or_default();
    let line = out
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix(PATH_MARKER))
        .ok_or_else(|| format!("{shell} {flag} 没有输出 PATH"))?;
    Ok(std::env::split_paths(line)
        .filter(|p| p.is_absolute())
        .collect())
}

/// Windows：没有登录 shell 这回事，新开终端拿到的 PATH 就是注册表里的系统 + 用户 PATH。
#[cfg(windows)]
fn probe_user_path(_home: &Path) -> UserPath {
    match crate::platform::registry_path_dirs() {
        Ok(dirs) => UserPath {
            stable: dirs.clone(),
            dirs,
            error: None,
        },
        Err(error) => UserPath {
            dirs: std::env::var_os("PATH")
                .map(|p| std::env::split_paths(&p).collect())
                .unwrap_or_default(),
            stable: Vec::new(),
            error: Some(error),
        },
    }
}

#[cfg(not(windows))]
fn user_shell(configured: Option<&str>, macos: bool) -> &str {
    configured
        .filter(|s| s.starts_with('/'))
        .unwrap_or(if macos { "/bin/zsh" } else { "/bin/bash" })
}

#[cfg(not(windows))]
fn probe_user_path(home: &Path) -> UserPath {
    let configured = std::env::var("SHELL").ok();
    let shell = user_shell(configured.as_deref(), cfg!(target_os = "macos"));
    let forms: Vec<Result<Vec<PathBuf>, String>> = ["-lic", "-lc", "-c"]
        .iter()
        .map(|flag| shell_path(shell, flag, home))
        .collect();
    let ok: Vec<&Vec<PathBuf>> = forms.iter().filter_map(|r| r.as_ref().ok()).collect();
    if ok.is_empty() {
        let fallback: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        let error = forms.iter().find_map(|r| r.as_ref().err().cloned());
        return UserPath {
            dirs: fallback,
            stable: Vec::new(),
            error,
        };
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    for list in &ok {
        for dir in list.iter() {
            if !dirs.contains(dir) {
                dirs.push(dir.clone());
            }
        }
    }
    let stable = if ok.len() == forms.len() {
        dirs.iter()
            .filter(|dir| ok.iter().all(|list| list.contains(dir)))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    let error = (ok.len() != forms.len()).then(|| {
        forms
            .iter()
            .find_map(|r| r.as_ref().err().cloned())
            .unwrap_or_default()
    });
    UserPath {
        dirs,
        stable,
        error,
    }
}

/// 用户 PATH；同一进程内缓存几分钟（桌面应用反复查询时不必每次都启动三个 shell）。
/// `refresh` 为真时重新探测（程序页的「重新扫描」）。
pub fn user_path(ctx: &Context, refresh: bool) -> UserPath {
    static CACHE: OnceLock<Mutex<BTreeMap<PathBuf, (Instant, UserPath)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Some(forced) = std::env::var_os("AGENT_MANAGER_USER_PATH") {
        // 验收与测试：显式给定用户 PATH，不启动 shell
        let dirs: Vec<PathBuf> = std::env::split_paths(&forced).collect();
        return UserPath {
            stable: dirs.clone(),
            dirs,
            error: None,
        };
    }
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if !refresh {
        if let Some((at, value)) = guard.get(&ctx.home_dir) {
            if at.elapsed() < CACHE_TTL {
                return value.clone();
            }
        }
    }
    let value = probe_user_path(&ctx.home_dir);
    guard.insert(ctx.home_dir.clone(), (Instant::now(), value.clone()));
    value
}

/// 命令在 PATH 上的全部位置（按 PATH 顺序；第一个就是会被用到的那个）。
pub fn find_all(path: &UserPath, bin: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for dir in &path.dirs {
        // Windows 上 `wecom-cli` 实际是 `wecom-cli.cmd` 等，按 PATHEXT 取这个目录里第一个
        let Some(candidate) = crate::platform::command_candidates(dir, bin)
            .into_iter()
            .find(|p| is_executable(p))
        else {
            continue;
        };
        if !out.iter().any(|p| same_file(p, &candidate)) {
            out.push(candidate);
        }
    }
    out
}

fn is_executable(path: &Path) -> bool {
    crate::platform::is_executable(path)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canon(), b.canon()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// 一处安装：命令所在位置、真实位置、归属与版本。
#[derive(Clone, Debug)]
pub struct Located {
    pub path: PathBuf,
    pub real: PathBuf,
    /// managed / npm-global / uv-tool / homebrew / app / agent / system / unknown
    pub owner: &'static str,
    /// 归属能认出的包标识（`npm:@wecom/cli`、`pypi:example-mcp`、`brew:jq`）。
    pub program: Option<String>,
    pub version: Option<String>,
    /// 包目录（npm 全局的 node_modules/<包>、uv tool 的环境目录、Homebrew 的版本目录）。
    pub package_dir: Option<PathBuf>,
    /// `.app` 名称（归属为 app 时）。
    pub app: Option<String>,
}

fn read_json(path: &Path) -> Option<Value> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > 1024 * 1024 {
        return None;
    }
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// 智能体自己的程序（Claude Code、Codex 的官方安装位置）：不在程序管理范围内。
fn agent_owned(real: &Path, home: &Path) -> bool {
    [".local/share/claude", ".codex/packages", ".claude/local"]
        .iter()
        .any(|rel| real.starts_with(home.join(rel)))
}

pub fn classify(ctx: &Context, path: &Path) -> Located {
    // Windows 上命令常是转发脚本（npm 的 cmd-shim、本工具的入口）：顺着它找到真正的程序
    let followed = crate::platform::cmd_shim_target(path).unwrap_or_else(|| path.to_path_buf());
    let real = followed.canon().unwrap_or(followed);
    let base = Located {
        path: path.to_path_buf(),
        real: real.clone(),
        owner: "unknown",
        program: None,
        version: None,
        package_dir: None,
        app: None,
    };
    // 统一用 `/` 匹配下面的目录特征（Windows 路径用 `\`）
    let text = real.to_string_lossy().replace('\\', "/");
    let root = ctx.data_dir.join("programs");
    let root = root.canon().unwrap_or(root);
    if let Ok(rest) = real.strip_prefix(&root) {
        // programs/<类别>/<名>/<版本>/...：认回包标识与版本
        let parts: Vec<String> = rest
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        let (program, version, dir) = match parts.as_slice() {
            [scheme, name, version, ..] if matches!(scheme.as_str(), "npm" | "pypi" | "git") => (
                Some(format!("{scheme}:{}", name.replace('~', "/"))),
                Some(version.clone()),
                Some(root.join(scheme).join(name)),
            ),
            _ => (None, None, None),
        };
        return Located {
            owner: "managed",
            program,
            version,
            package_dir: dir,
            ..base
        };
    }
    if agent_owned(&real, &ctx.home_dir) {
        return Located {
            owner: "agent",
            ..base
        };
    }
    // npm 全局：Unix 在 <prefix>/lib/node_modules，Windows 在 <prefix>\node_modules（默认 %APPDATA%\npm）
    let npm_global = if cfg!(windows) {
        "/node_modules/"
    } else {
        "/lib/node_modules/"
    };
    if let Some(pos) = text.find(npm_global) {
        let rest = &text[pos + npm_global.len()..];
        let mut parts = rest.split('/');
        let first = parts.next().unwrap_or_default();
        let name = if first.starts_with('@') {
            format!("{first}/{}", parts.next().unwrap_or_default())
        } else {
            first.to_string()
        };
        let dir = PathBuf::from(&text[..pos + npm_global.len()]).join(&name);
        let version = read_json(&dir.join("package.json"))
            .and_then(|v| v.get("version").and_then(Value::as_str).map(str::to_string));
        return Located {
            owner: "npm-global",
            program: Some(format!("npm:{name}")),
            version,
            package_dir: Some(dir),
            ..base
        };
    }
    #[cfg(unix)]
    if let Some((tool, dir)) = super::uv_custom_root::locate(path, &real) {
        return Located {
            owner: "uv-tool",
            program: Some(format!("pypi:{tool}")),
            version: uv_tool_version(&dir, &tool),
            package_dir: Some(dir),
            ..base
        };
    }
    if let Some((pos, marker)) = ["/uv/tools/", "/uv/data/tools/"]
        .into_iter()
        .find_map(|marker| text.find(marker).map(|pos| (pos, marker)))
    {
        let rest = &text[pos + marker.len()..];
        let tool = rest.split('/').next().unwrap_or_default().to_string();
        let dir = PathBuf::from(&text[..pos + marker.len()]).join(&tool);
        let version = uv_tool_version(&dir, &tool);
        return Located {
            owner: "uv-tool",
            program: Some(format!("pypi:{tool}")),
            version,
            package_dir: Some(dir),
            ..base
        };
    }
    #[cfg(windows)]
    if let Some((tool, dir)) = windows_uv_copy(ctx, path) {
        return Located {
            owner: "uv-tool",
            program: Some(format!("pypi:{tool}")),
            version: uv_tool_version(&dir, &tool),
            package_dir: Some(dir),
            ..base
        };
    }
    if let Some(pos) = text.find("/Cellar/") {
        let mut parts = text[pos + "/Cellar/".len()..].split('/');
        let formula = parts.next().unwrap_or_default().to_string();
        let version = parts.next().map(str::to_string);
        let dir = PathBuf::from(&text[..pos + "/Cellar/".len()]).join(&formula);
        return Located {
            owner: "homebrew",
            program: Some(format!("brew:{formula}")),
            version,
            package_dir: Some(dir),
            ..base
        };
    }
    if let Some(pos) = text.find(".app/Contents/") {
        let app = text[..pos]
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string();
        return Located {
            owner: "app",
            app: Some(app),
            ..base
        };
    }
    if ["/usr/bin/", "/bin/", "/usr/sbin/", "/sbin/", "/System/"]
        .iter()
        .any(|prefix| text.starts_with(prefix))
    {
        return Located {
            owner: "system",
            ..base
        };
    }
    base
}

fn uv_tool_version(dir: &Path, tool: &str) -> Option<String> {
    super::uv_metadata::version(dir, tool).ok()
}

/// Windows uv copies entry points. A receipt path alone does not prove ownership.
pub(super) fn uv_owned_copy(dir: &Path, path: &Path) -> bool {
    use std::io::Read;
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
        return false;
    }
    let receipt = dir.join("uv-receipt.toml");
    if std::fs::metadata(&receipt).map_or(true, |meta| meta.len() > 64 * 1024) {
        return false;
    }
    let Some(receipt) = std::fs::read_to_string(receipt)
        .ok()
        .and_then(|text| text.parse::<toml::Value>().ok())
    else {
        return false;
    };
    let Some(entries) = receipt
        .get("tool")
        .and_then(|tool| tool.get("entrypoints"))
        .and_then(toml::Value::as_array)
    else {
        return false;
    };
    let Ok(actual) = path.canon() else {
        return false;
    };
    if !entries.iter().any(|entry| {
        entry
            .get("install-path")
            .and_then(toml::Value::as_str)
            .and_then(|value| Path::new(value).canon().ok())
            .is_some_and(|value| value == actual)
    }) {
        return false;
    }
    let Some(name) = path.file_name() else {
        return false;
    };
    let source = dir.join("Scripts").join(name);
    if std::fs::metadata(&source)
        .map_or(true, |meta| !meta.is_file() || meta.len() != metadata.len())
    {
        return false;
    }
    let (Ok(mut source), Ok(mut installed)) =
        (std::fs::File::open(source), std::fs::File::open(path))
    else {
        return false;
    };
    let (mut a, mut b) = ([0; 8192], [0; 8192]);
    loop {
        let (Ok(a_len), Ok(b_len)) = (source.read(&mut a), installed.read(&mut b)) else {
            return false;
        };
        if a_len != b_len || a[..a_len] != b[..b_len] {
            return false;
        }
        if a_len == 0 {
            return true;
        }
    }
}

#[cfg(windows)]
fn windows_uv_copy(ctx: &Context, path: &Path) -> Option<(String, PathBuf)> {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return None;
    }
    let root = std::env::var_os("UV_TOOL_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| ctx.home_dir.join("AppData/Roaming"))
                .join("uv/data/tools")
        });
    if !root.is_absolute() {
        return None;
    }
    for entry in std::fs::read_dir(root).ok()?.take(512).flatten() {
        if uv_owned_copy(&entry.path(), path) {
            return Some((entry.file_name().to_str()?.to_string(), entry.path()));
        }
    }
    None
}

/// 一个包目录里的版本（npm 的 `package.json`）。
pub fn read_version_at(dir: &Path) -> Option<String> {
    read_json(&dir.join("package.json"))
        .and_then(|v| v.get("version").and_then(Value::as_str).map(str::to_string))
}

/// npm 包提供的全部命令（`package.json` 的 `bin`）。
pub fn npm_bins(package_dir: &Path) -> Vec<String> {
    let Some(pkg) = read_json(&package_dir.join("package.json")) else {
        return Vec::new();
    };
    match pkg.get("bin") {
        Some(Value::String(_)) => pkg
            .get("name")
            .and_then(Value::as_str)
            .map(|name| vec![name.rsplit('/').next().unwrap_or(name).to_string()])
            .unwrap_or_default(),
        Some(Value::Object(map)) => map.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

impl Located {
    pub fn to_json(&self, home: &Path) -> Value {
        let tilde = |p: &Path| match p.strip_prefix(home) {
            Ok(rest) => format!("~/{}", crate::platform::slash(&rest)),
            Err(_) => p.to_string_lossy().to_string(),
        };
        json!({
            "path": tilde(&self.path),
            "real": tilde(&self.real),
            "owner": self.owner,
            "program": self.program,
            "version": self.version,
            "app": self.app,
            "package_dir": self.package_dir.as_ref().map(|d| tilde(d)),
        })
    }
}

#[cfg(all(test, unix))]
mod capture_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn missing_or_unusable_shell_uses_the_platform_default() {
        for configured in [None, Some(""), Some("bash")] {
            assert_eq!(user_shell(configured, false), "/bin/bash");
            assert_eq!(user_shell(configured, true), "/bin/zsh");
        }
        assert_eq!(user_shell(Some("/custom/shell"), false), "/custom/shell");
    }

    #[test]
    fn shell_path_deadline_covers_descendant_output_after_shell_exit() {
        let root = crate::process::capture_test_support::root("shell-path");
        let shell = root.join("shell");
        std::fs::write(
            &shell,
            r#"#!/bin/sh
(printf ready > "$HOME/ready"; sleep 11; printf survived > "$HOME/late") &
printf exited > "$HOME/parent-exited"
printf '\n__AGENT_MANAGER_PATH__/usr/bin:/bin\n'
"#,
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        let start = Instant::now();
        let result = shell_path(shell.to_str().unwrap(), "-c", &root);
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(10),
            "PATH probe waited for descendant: {elapsed:?}"
        );
        assert!(result.unwrap_err().contains("超时"));
        assert!(root.join("ready").exists());
        assert!(root.join("parent-exited").exists());
        std::thread::sleep(Duration::from_secs(12).saturating_sub(start.elapsed()));
        assert!(
            !root.join("late").exists(),
            "PATH probe left its descendant running"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shell_path_drains_verbose_shell_output_without_blocking() {
        let root = crate::process::capture_test_support::root("shell-output");
        let shell = root.join("shell");
        std::fs::write(
            &shell,
            r#"#!/bin/sh
printf '\n__AGENT_MANAGER_PATH__/usr/bin:/bin\n'
head -c 524288 /dev/zero
"#,
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            shell_path(shell.to_str().unwrap(), "-c", &root).unwrap(),
            vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")]
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod uv_tests {
    use super::*;
    #[test]
    fn windows_uv_layout_and_copied_entry_require_receipt_and_matching_bytes() {
        let root = std::env::temp_dir().join(format!("am-uv-copy-{}", crate::store::uuid_v4()));
        let dir = root.join("uv/data/tools/tool-a");
        let source = dir.join("Scripts/tool-a.exe");
        let installed = root.join("bin/tool-a.exe");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        let site = dir.join(if cfg!(windows) {
            "Lib/site-packages"
        } else {
            "lib/python3.13/site-packages"
        });
        std::fs::create_dir_all(site.join("tool_a-1.2.3.dist-info")).unwrap();
        std::fs::write(
            site.join("tool_a-1.2.3.dist-info/METADATA"),
            "Metadata-Version: 2.4\nName: tool-a\nVersion: 1.2.3\n\n",
        )
        .unwrap();
        std::fs::write(&source, b"original entry").unwrap();
        std::fs::copy(&source, &installed).unwrap();
        assert!(!uv_owned_copy(&dir, &installed));
        std::fs::write(
            dir.join("uv-receipt.toml"),
            format!(
                "[tool]\nentrypoints = [{{ name = 'tool-a', install-path = {} }}]\n",
                serde_json::to_string(&installed.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();
        assert!(uv_owned_copy(&dir, &installed));
        assert_eq!(
            uv_tool_version(&dir.canon().unwrap(), "tool-a").as_deref(),
            Some("1.2.3")
        );
        std::fs::write(&installed, b"modified entry").unwrap();
        assert!(!uv_owned_copy(&dir, &installed));
        std::fs::remove_dir_all(root).unwrap();
    }
}
