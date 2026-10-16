//! 平台差异集中在这里，各域不再各写一份 `cfg`：
//! - 主目录：Unix 用 `HOME`；Windows 默认没有 `HOME`，回落 `USERPROFILE`。
//! - 目录链接：Unix 用符号链接；Windows 普通用户默认建不了符号链接（要开发者模式或管理员），
//!   先试符号链接，不行就建目录联接（junction，普通用户可建，只能指向本机的绝对路径）。
//! - 删除链接：Windows 上指向目录的链接（含联接）要用 `remove_dir` 删，只删链接本身。
//! - 可执行文件：Unix 看执行位；Windows 看扩展名（`PATHEXT`，如 `.exe`、`.cmd`），
//!   查找命令时按 `PATHEXT` 补扩展名（`claude` → `claude.cmd`）。

use std::path::{Path, PathBuf};

/// 用户主目录。没设置、为空或是相对路径时为 None：相对路径会随启动目录变，
/// 拿它拼路径会把配置和数据写进启动时所在的目录。
pub fn home_dir() -> Option<PathBuf> {
    home_from(|name| std::env::var_os(name))
}

fn home_from(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    let usable = |name: &str| {
        var(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    if let Some(home) = var("HOME").filter(|value| !value.is_empty()) {
        // HOME 设了但不可用就报不可用，不悄悄改用别的变量
        return Some(PathBuf::from(home)).filter(|path| path.is_absolute());
    }
    if cfg!(windows) {
        return usable("USERPROFILE");
    }
    None
}

/// 建一个指向目录的链接（`link` 不能已存在）。
pub fn link_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_dir(target, link) {
            Ok(()) => Ok(()),
            // 没有建符号链接的权限：改建联接。联接只认绝对路径，相对目标按链接所在目录展开
            Err(_) => {
                let absolute = if target.is_absolute() {
                    target.to_path_buf()
                } else {
                    link.parent().unwrap_or(Path::new(".")).join(target)
                };
                junction::create(&absolute, link)
            }
        }
    }
}

/// 建一个指向文件的链接（`link` 不能已存在）。Windows 没有权限建文件符号链接时退回复制一份。
pub fn link_file(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link).or_else(|_| {
            let absolute = if target.is_absolute() {
                target.to_path_buf()
            } else {
                link.parent().unwrap_or(Path::new(".")).join(target)
            };
            std::fs::copy(absolute, link).map(|_| ())
        })
    }
}

/// 删掉一个链接本身（不动它指向的内容）。
pub fn remove_link(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        if std::fs::symlink_metadata(path)?
            .file_type()
            .is_symlink_dir()
        {
            return std::fs::remove_dir(path);
        }
    }
    std::fs::remove_file(path)
}

/// 用新建好的临时链接 `tmp` 替换 `dest`。Unix 上一次 rename 原子替换；Windows 不允许 rename
/// 覆盖目录（指向目录的链接与联接都算），`dest` 是链接时先删掉它再改名——中间有极短的空档。
/// `dest` 不是链接（真实目录或文件）时不删，交给 rename 报错。
pub fn replace_link(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        if std::fs::symlink_metadata(dest).is_ok_and(|m| m.file_type().is_symlink()) {
            remove_link(dest)?;
        }
    }
    std::fs::rename(tmp, dest)
}

/// 规范化路径（解析链接与 `..`）。Windows 上标准库给出 `\\?\C:\...` 这种带前缀的路径，
/// 与普通路径比较不相等、写进配置也不对；能安全去掉时去掉（`dunce`），Unix 上同标准库。
pub trait Canon {
    fn canon(&self) -> std::io::Result<PathBuf>;
}

impl<P: AsRef<Path> + ?Sized> Canon for P {
    fn canon(&self) -> std::io::Result<PathBuf> {
        dunce::canonicalize(self.as_ref())
    }
}

/// 相对路径用 `/` 连起来，给 `~/…` 一类显示用：Windows 上系统路径用 `\`，混在 `~/` 后面既难看
/// 又与其他平台不一致；Windows 本身也认 `/`。
pub fn slash(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// 在 `base` 下按 `/` 分段拼上相对路径（`~/a/b` 一类配置写法）：Windows 上整段带 `/` 的字符串
/// 直接 join 会得到 `C:\\home\\a/b` 这种混合分隔符。
pub fn join_slash(base: &Path, rel: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        out.push(part);
    }
    out
}

/// 本工具调用的 git：关掉换行转换（经 `GIT_CONFIG_COUNT` 环境变量，git 2.31 起支持；只对这一次
/// 调用生效，不改用户的 git 配置）。Git for Windows 默认检出时把 LF 改成 CRLF，下载的技能与上游不再逐字节一致，
/// 技能里的 `.sh` 在 Git Bash 里也会因为多出的 `\r` 跑不起来。
pub fn git_command() -> std::process::Command {
    let mut command = crate::process::command("git");
    command
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.autocrlf")
        .env("GIT_CONFIG_VALUE_0", "false");
    command
}

/// 读链接指向的位置；Windows 上去掉 `\\?\` 这类前缀，便于与普通路径比较。
pub fn read_link(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    let target = std::fs::read_link(path.as_ref())?;
    #[cfg(windows)]
    {
        let text = target.to_string_lossy();
        if let Some(rest) = text
            .strip_prefix(r"\\?\")
            .or_else(|| text.strip_prefix(r"\??\"))
        {
            return Ok(PathBuf::from(rest));
        }
    }
    Ok(target)
}

/// 这个文件能不能当命令执行。
pub fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(windows)]
    {
        path.extension()
            .map(|ext| {
                let ext = format!(".{}", ext.to_string_lossy()).to_ascii_uppercase();
                path_exts().iter().any(|known| *known == ext)
            })
            .unwrap_or(false)
    }
}

/// Windows 认作可执行的扩展名（大写，带点），按 `PATHEXT` 的顺序。
#[cfg(windows)]
fn path_exts() -> Vec<String> {
    std::env::var("PATHEXT")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.to_ascii_uppercase())
        .collect()
}

/// 命令名在一个目录里可能对应的文件：Unix 就是同名文件；Windows 先按原名（已带扩展名时），
/// 再按 `PATHEXT` 逐个补扩展名。
pub fn command_candidates(dir: &Path, name: &str) -> Vec<PathBuf> {
    #[cfg(unix)]
    {
        vec![dir.join(name)]
    }
    #[cfg(windows)]
    {
        let mut out = Vec::new();
        if Path::new(name).extension().is_some() {
            out.push(dir.join(name));
        }
        for ext in path_exts() {
            out.push(dir.join(format!("{name}{}", ext.to_ascii_lowercase())));
        }
        out
    }
}

/// 在这些目录里按顺序找命令，返回第一个能执行的文件。
pub fn find_command<I: IntoIterator<Item = PathBuf>>(dirs: I, name: &str) -> Option<PathBuf> {
    dirs.into_iter()
        .flat_map(|dir| command_candidates(&dir, name))
        .find(|path| is_executable(path))
}

/// 文件名去掉 Windows 的可执行扩展名，得到用户敲的命令名（`claude.cmd` → `claude`）。
pub fn command_name(file_name: &str) -> &str {
    #[cfg(windows)]
    {
        if let Some((stem, ext)) = file_name.rsplit_once('.') {
            let ext = format!(".{ext}").to_ascii_uppercase();
            if path_exts().contains(&ext) {
                return stem;
            }
        }
    }
    file_name
}

/// 把 `%VAR%` 换成环境变量的值（不存在的原样保留），与 Windows 展开 REG_EXPAND_SZ 的方式一致。
pub fn expand_env(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) => out.push_str(&value),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Windows：新开的终端会拿到的 PATH——注册表里系统的在前、用户的在后，`%VAR%` 已展开。
/// 桌面应用启动后用户改过的 PATH，本进程的环境变量里看不到，所以直接读注册表。
#[cfg(windows)]
pub fn registry_path_dirs() -> Result<Vec<PathBuf>, String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let read = |root, key: &str| -> Option<String> {
        RegKey::predef(root)
            .open_subkey(key)
            .ok()?
            .get_value::<String, _>("Path")
            .ok()
    };
    let machine = read(
        HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
    );
    let user = read(HKEY_CURRENT_USER, "Environment");
    if machine.is_none() && user.is_none() {
        return Err("注册表里读不到 PATH".to_string());
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    for part in [machine, user].into_iter().flatten() {
        for item in part.split(';').filter(|item| !item.trim().is_empty()) {
            let dir = PathBuf::from(expand_env(item.trim()));
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    Ok(dirs)
}

/// 本工具写的命令转发脚本第二行的标记，后面是真正的程序位置。
const SHIM_HEX_MARK: &str = "rem agent-manager link-utf8-hex: ";
const SHIM_MARK: &str = "rem agent-manager link: ";

/// 命令入口的文件名：Unix 就是命令名（放符号链接）；Windows 为 `<命令>.cmd`（放转发脚本）。
pub fn command_link_name(bin: &str) -> String {
    if cfg!(windows) {
        format!("{bin}.cmd")
    } else {
        bin.to_string()
    }
}

/// 写一个转发脚本：在 cmd / PowerShell 里敲命令名时运行 `target` 并原样传参（Windows 上代替
/// 指向命令文件的符号链接——npm 的 `.cmd` 按自身所在目录找文件，链接或复制到别处都会失效）。
pub fn write_cmd_shim(target: &Path, path: &Path) -> std::io::Result<()> {
    std::fs::write(path, cmd_shim_text(target)?)
}

pub(crate) fn cmd_shim_text(target: &Path) -> std::io::Result<String> {
    let target = target.to_string_lossy();
    if target.chars().any(char::is_control) || target.contains('"') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Command targets cannot contain quotes or control characters",
        ));
    }
    let encoded = target
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!(
        "@echo off\r\n{SHIM_HEX_MARK}{encoded}\r\nsetlocal DisableDelayedExpansion\r\n\"{}\" %*\r\n",
        target.replace('%', "%%")
    ))
}

/// 命令转发脚本指向的真实文件：本工具写的（第二行标记），或 npm 生成的 cmd-shim
/// （`"%dp0%\node_modules\...\x.js"`，按脚本所在目录展开）。不是转发脚本时为 None。
pub fn cmd_shim_target(path: &Path) -> Option<PathBuf> {
    let is_cmd = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd"));
    if !is_cmd || std::fs::metadata(path).ok()?.len() > 64 * 1024 {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    if let Some(encoded) = text
        .lines()
        .find_map(|line| line.strip_prefix(SHIM_HEX_MARK))
    {
        if encoded.len() % 2 != 0 || !encoded.bytes().all(|ch| ch.is_ascii_hexdigit()) {
            return None;
        }
        let bytes = (0..encoded.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&encoded[i..i + 2], 16))
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        return Some(PathBuf::from(String::from_utf8(bytes).ok()?));
    }
    if let Some(target) = text.lines().find_map(|line| line.strip_prefix(SHIM_MARK)) {
        return Some(PathBuf::from(target.trim()));
    }
    let re = regex::Regex::new(r#""%(?:dp0%|~dp0)\\?([^"%]+)""#).expect("固定正则");
    let rel = re.captures_iter(&text).last()?.get(1)?.as_str().to_string();
    let mut out = path.parent()?.to_path_buf();
    for part in rel.split(['\\', '/']).filter(|p| !p.is_empty()) {
        out.push(part);
    }
    Some(out)
}

/// 命令入口指向哪里：符号链接读链接；Windows 的转发脚本读脚本里记的位置。都不是时为 None。
pub fn command_link_target(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() {
        return read_link(path).ok();
    }
    cmd_shim_target(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_must_be_an_absolute_path() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| std::ffi::OsString::from(*value))
            }
        };
        let absolute = if cfg!(windows) {
            r"C:\Users\me"
        } else {
            "/Users/me"
        };
        assert_eq!(
            home_from(|name| (name == "HOME").then(|| absolute.into())),
            Some(PathBuf::from(absolute))
        );
        assert_eq!(home_from(env(&[("HOME", "relative/home")])), None);
        assert_eq!(home_from(env(&[("HOME", ".")])), None);
        assert_eq!(home_from(env(&[("HOME", "")])), None);
        assert_eq!(home_from(env(&[])), None);
    }

    #[test]
    fn 目录链接_建_读_删只删链接本身() {
        let root = std::env::temp_dir().join(format!("am-platform-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let target = root.join("target");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("SKILL.md"), "x").unwrap();
        let link = root.join("link");
        link_dir(&target, &link).unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_to_string(link.join("SKILL.md")).unwrap(), "x");
        assert_eq!(
            read_link(&link).unwrap().canonicalize().unwrap(),
            target.canonicalize().unwrap()
        );
        remove_link(&link).unwrap();
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert!(target.join("SKILL.md").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn 展开百分号变量_不存在的原样保留() {
        std::env::set_var("AM_PLATFORM_TEST_DIR", "C:\\x");
        assert_eq!(
            expand_env("%AM_PLATFORM_TEST_DIR%\\bin;%AM_NOPE%;50%"),
            "C:\\x\\bin;%AM_NOPE%;50%"
        );
    }

    #[test]
    fn 转发脚本_本工具写的与_npm_生成的都能认出目标() {
        let root = std::env::temp_dir().join(format!("am-platform-shim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let ours = root.join("tool.cmd");
        write_cmd_shim(
            Path::new(r"C:\am\programs\npm\x\current\node_modules\.bin\tool.cmd"),
            &ours,
        )
        .unwrap();
        assert_eq!(
            cmd_shim_target(&ours),
            Some(PathBuf::from(
                r"C:\am\programs\npm\x\current\node_modules\.bin\tool.cmd"
            ))
        );
        std::fs::write(
            &ours,
            format!("@echo off\r\n{SHIM_MARK}C:\\legacy\\tool.cmd\r\n"),
        )
        .unwrap();
        assert_eq!(
            cmd_shim_target(&ours),
            Some(PathBuf::from(r"C:\legacy\tool.cmd"))
        );
        // npm cmd-shim（节选）：目标按脚本所在目录展开
        let npm = root.join("wecom-cli.cmd");
        std::fs::write(&npm, "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\node_modules\\@wecom\\cli\\bin\\wecom.js\" %*\r\n").unwrap();
        assert_eq!(
            cmd_shim_target(&npm),
            Some(
                root.join("node_modules")
                    .join("@wecom")
                    .join("cli")
                    .join("bin")
                    .join("wecom.js")
            )
        );
        assert_eq!(cmd_shim_target(&root.join("missing.cmd")), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn native_cmd_shim_preserves_metacharacter_targets_arguments_and_exit_status() {
        let root = std::env::temp_dir().join(format!("am-shim-{}", crate::store::uuid_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("probe.rs");
        std::fs::write(&source, r#"fn main() { let args: Vec<_> = std::env::args().skip(1).collect(); std::fs::write(&args[0], format!("{:?}", &args[1..])).unwrap(); println!("CHILD"); std::process::exit(37); }"#).unwrap();
        let target = root.join("program %AM_SHIM_HACK% ! ^ (x) & echo INJECTED & rem x.exe");
        assert!(std::process::Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&target)
            .status()
            .unwrap()
            .success());
        let shim = root.join("entry.cmd");
        write_cmd_shim(&target, &shim).unwrap();
        assert_eq!(cmd_shim_target(&shim), Some(target));
        let output_file = root.join("args.txt");
        let command = crate::native_launch::windows_program_command(
            shim.to_str().unwrap(),
            &[
                output_file.to_string_lossy().into_owned(),
                "literal %NO_SUCH_VAR% & ! ^ (x)".into(),
                "".into(),
            ],
            &[("AM_SHIM_HACK".into(), "wrong".into())],
            None,
        )
        .unwrap();
        let output = std::process::Command::new(
            Path::new(&std::env::var("SystemRoot").unwrap())
                .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
        )
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("{command}; exit $LASTEXITCODE"),
        ])
        .env("PSModuleAnalysisCachePath", "NUL")
        .output()
        .unwrap();
        assert_eq!(output.status.code(), Some(37), "{:?}", output);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("INJECTED"));
        assert_eq!(
            std::fs::read_to_string(output_file).unwrap(),
            format!("{:?}", ["literal %NO_SUCH_VAR% & ! ^ (x)", ""])
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 按平台规则找命令() {
        let root = std::env::temp_dir().join(format!("am-platform-cmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        #[cfg(unix)]
        let file = {
            use std::os::unix::fs::PermissionsExt;
            let file = root.join("tool");
            std::fs::write(&file, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
            file
        };
        #[cfg(windows)]
        let file = {
            let file = root.join("tool.cmd");
            std::fs::write(&file, "@echo off\r\n").unwrap();
            file
        };
        assert_eq!(find_command([root.clone()], "tool"), Some(file.clone()));
        assert_eq!(find_command([root.clone()], "missing"), None);
        assert_eq!(
            command_name(&file.file_name().unwrap().to_string_lossy()),
            "tool"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
