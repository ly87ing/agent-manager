//! Read legacy OS login entries and remove them only after an explicit request.

use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use tauri::Manager;

pub(crate) fn get(app: &tauri::AppHandle) -> Result<bool, String> {
    let executable = executable(app)?;
    status(&app.config().identifier, &executable).map_err(|error| error.to_string())
}

pub(crate) fn set(app: &tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    let executable = executable(app)?;
    set_status(&app.config().identifier, &executable, enabled)
        .map_err(|error| error.to_string())?;
    let actual = get(app)?;
    if actual != enabled {
        return Err("开机启动设置未生效，请检查系统的登录项设置".into());
    }
    Ok(actual)
}

fn executable(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    if let Some(path) = app.env().appimage {
        return Ok(PathBuf::from(path));
    }
    #[cfg(not(target_os = "linux"))]
    let _ = app;
    std::env::current_exe().map_err(|error| error.to_string())
}

#[cfg(not(windows))]
fn entry_path(identifier: &str) -> std::io::Result<PathBuf> {
    let home = agent_manager_engine::platform::home_dir()
        .ok_or_else(|| std::io::Error::other("找不到绝对路径的用户主目录"))?;
    #[cfg(target_os = "macos")]
    return Ok(home
        .join("Library/LaunchAgents")
        .join(format!("{identifier}.plist")));
    #[cfg(target_os = "linux")]
    {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        if !config.is_absolute() {
            return Err(std::io::Error::other("XDG_CONFIG_HOME 必须是绝对路径"));
        }
        Ok(config
            .join("autostart")
            .join(format!("{identifier}.desktop")))
    }
}

#[cfg(not(windows))]
fn status(identifier: &str, executable: &Path) -> std::io::Result<bool> {
    file_status(
        &entry_path(identifier)?,
        &entry_contents(identifier, executable)?,
    )
}

#[cfg(not(windows))]
fn set_status(identifier: &str, executable: &Path, enabled: bool) -> std::io::Result<()> {
    set_file_status(
        &entry_path(identifier)?,
        &entry_contents(identifier, executable)?,
        enabled,
    )
}

#[cfg(not(windows))]
fn file_status(path: &Path, expected: &[u8]) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
        Ok(meta) if !meta.file_type().is_file() => {
            Err(std::io::Error::other("开机启动条目不是普通文件，未改动"))
        }
        Ok(_) if std::fs::read(path)? == expected => Ok(true),
        Ok(_) => Err(std::io::Error::other(
            "开机启动条目已被其他程序修改，未覆盖；请检查系统的登录项设置",
        )),
    }
}

#[cfg(not(windows))]
fn set_file_status(path: &Path, expected: &[u8], enabled: bool) -> std::io::Result<()> {
    if enabled {
        return Err(std::io::Error::other("开机启动已移除，仅支持移除旧登录项"));
    }
    if !file_status(path, expected)? {
        return Ok(());
    }
    // Re-read immediately before deletion; never remove an externally replaced entry.
    if file_status(path, expected)? {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn entry_contents(identifier: &str, executable: &Path) -> std::io::Result<Vec<u8>> {
    let mut entry = plist::Dictionary::new();
    entry.insert("Label".into(), identifier.into());
    entry.insert(
        "ProgramArguments".into(),
        plist::Value::Array(vec![executable.to_string_lossy().as_ref().into()]),
    );
    entry.insert("RunAtLoad".into(), true.into());
    // launchd must not kill the replacement process when the updater restarts the app.
    entry.insert("AbandonProcessGroup".into(), true.into());
    let mut bytes = Vec::new();
    plist::Value::Dictionary(entry)
        .to_writer_xml(&mut bytes)
        .map_err(std::io::Error::other)?;
    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn entry_contents(_identifier: &str, executable: &Path) -> std::io::Result<Vec<u8>> {
    let path = executable
        .to_str()
        // GIO checks the executable before expanding field codes, including %%.
        // '=' is forbidden in executable paths by the Desktop Entry specification.
        .filter(|path| !path.contains(['%', '=']) && !path.chars().any(char::is_control))
        .ok_or_else(|| std::io::Error::other("启动路径包含不支持的字符，请移动应用后重试"))?;
    let mut escaped = String::new();
    for character in path.chars() {
        match character {
            '\\' => escaped.push_str("\\\\\\\\"),
            '"' | '`' | '$' => {
                escaped.push_str("\\\\");
                escaped.push(character);
            }
            _ => escaped.push(character),
        }
    }
    Ok(format!("[Desktop Entry]\nType=Application\nName=Agent Manager\nExec=\"{escaped}\"\nTerminal=false\n").into_bytes())
}

#[cfg(windows)]
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

#[cfg(windows)]
fn registered(identifier: &str, executable: &Path) -> std::io::Result<bool> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey(RUN) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    match key.get_value::<String, _>(identifier) {
        Ok(value) if value == format!("\"{}\"", executable.display()) => Ok(true),
        Ok(_) => Err(std::io::Error::other("开机启动条目指向其他位置，未覆盖")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn status(identifier: &str, executable: &Path) -> std::io::Result<bool> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    if !registered(identifier, executable)? {
        return Ok(false);
    }
    let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey(APPROVED) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error),
    };
    match key.get_raw_value(identifier) {
        Ok(value) => {
            Ok(value.bytes.len() >= 12 && value.bytes[4..12].iter().all(|byte| *byte == 0))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn set_status(identifier: &str, executable: &Path, enabled: bool) -> std::io::Result<()> {
    if enabled {
        return Err(std::io::Error::other("开机启动已移除，仅支持移除旧登录项"));
    }
    use winreg::{
        enums::{HKEY_CURRENT_USER, KEY_SET_VALUE},
        RegKey,
    };
    let current = registered(identifier, executable)?;
    let user = RegKey::predef(HKEY_CURRENT_USER);
    if current {
        user.open_subkey_with_flags(RUN, KEY_SET_VALUE)?
            .delete_value(identifier)?;
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use winreg::{
        enums::{HKEY_CURRENT_USER, REG_BINARY},
        RegKey, RegValue,
    };

    #[test]
    #[ignore = "Requires an isolated Windows user hive; run explicitly in Wine"]
    fn retired_enable_and_owned_registry_removal() {
        assert_eq!(
            std::env::var("AM_ISOLATED_WINDOWS_HIVE").as_deref(),
            Ok("1")
        );
        let identifier = format!("org.example.agent-manager.test-{}", std::process::id());
        let executable = Path::new(r"C:\Program Files\Agent Manager\app.exe");
        let user = RegKey::predef(HKEY_CURRENT_USER);
        let (run, _) = user.create_subkey(RUN).unwrap();
        let (approved, _) = user.create_subkey(APPROVED).unwrap();
        assert!(!status(&identifier, executable).unwrap());
        assert!(set_status(&identifier, executable, true).is_err());
        assert!(run.get_value::<String, _>(&identifier).is_err());
        run.set_value(&identifier, &format!("\"{}\"", executable.display()))
            .unwrap();
        let value: String = run.get_value(&identifier).unwrap();
        assert_eq!(value, r#""C:\Program Files\Agent Manager\app.exe""#);
        assert!(status(&identifier, executable).unwrap());
        approved
            .set_raw_value(
                &identifier,
                &RegValue {
                    vtype: REG_BINARY,
                    bytes: vec![3, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
                },
            )
            .unwrap();
        assert!(!status(&identifier, executable).unwrap());
        assert!(set_status(&identifier, executable, true).is_err());
        assert!(!status(&identifier, executable).unwrap());
        run.set_value(&identifier, &"external change").unwrap();
        assert!(set_status(&identifier, executable, false).is_err());
        assert_eq!(
            run.get_value::<String, _>(&identifier).unwrap(),
            "external change"
        );
        run.set_value(&identifier, &value).unwrap();
        set_status(&identifier, executable, false).unwrap();
        assert!(!status(&identifier, executable).unwrap());
        approved.delete_value(&identifier).unwrap();
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn unsupported_desktop_executable_paths_are_rejected() {
        for path in ["/opt/100%/app", "/opt/name=value/app", "/opt/new\nline/app"] {
            assert!(entry_contents("org.example.test", Path::new(path)).is_err());
        }
    }

    #[test]
    fn enabling_is_retired_and_owned_entry_removal_protects_external_changes() {
        let root = std::env::temp_dir().join(format!(
            "am-login-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("login/entry");
        let bytes = entry_contents(
            "org.example.agent-manager.test",
            Path::new("/Applications/Agent & Manager.app/Contents/MacOS/app"),
        )
        .unwrap();
        assert!(!file_status(&path, &bytes).unwrap());
        assert!(!root.exists(), "Queries must not create directories");
        assert!(set_file_status(&path, &bytes, true).is_err());
        assert!(
            !root.exists(),
            "Rejected enabling must not create directories"
        );
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(file_status(&path, &bytes).unwrap());
        assert!(set_file_status(&path, &bytes, true).is_err());
        #[cfg(target_os = "macos")]
        {
            let parsed = plist::Value::from_file(&path).unwrap();
            let entry = parsed.as_dictionary().unwrap();
            assert_eq!(entry["AbandonProcessGroup"].as_boolean(), Some(true));
            assert_eq!(
                entry["ProgramArguments"].as_array().unwrap()[0].as_string(),
                Some("/Applications/Agent & Manager.app/Contents/MacOS/app")
            );
        }
        std::fs::write(&path, b"external change").unwrap();
        assert!(set_file_status(&path, &bytes, true).is_err());
        assert!(set_file_status(&path, &bytes, false).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"external change");
        std::fs::write(&path, &bytes).unwrap();
        set_file_status(&path, &bytes, false).unwrap();
        assert!(!file_status(&path, &bytes).unwrap());
        std::os::unix::fs::symlink(root.join("other"), &path).unwrap();
        assert!(set_file_status(&path, &bytes, true).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
