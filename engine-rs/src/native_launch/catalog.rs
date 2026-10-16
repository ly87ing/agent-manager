//! Read application registrations without executing desktop entries or installers.
#[cfg(target_os = "linux")]
use crate::Context;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
pub(super) fn linux(ctx: &Context, apps: &mut Vec<Value>) {
    use std::collections::BTreeSet;
    let mut roots = vec![std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| ctx.home_dir.join(".local/share"))];
    roots.extend(
        std::env::var_os("XDG_DATA_DIRS")
            .map(|value| {
                std::env::split_paths(&value)
                    .filter(|path| path.is_absolute())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec!["/usr/local/share".into(), "/usr/share".into()]),
    );
    let path: Vec<_> = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();
    let mut seen = BTreeSet::new();
    for root in roots.into_iter().take(16) {
        let root = root.join("applications");
        let mut entries = Vec::new();
        desktop_files(&root, &root, 3, &mut 4096, &mut entries);
        entries.sort();
        for (id, file) in entries {
            if !seen.insert(id) {
                continue;
            }
            if std::fs::metadata(&file).map_or(true, |meta| meta.len() > 64 * 1024) {
                continue;
            }
            if let Some(app) = std::fs::read_to_string(&file)
                .ok()
                .and_then(|text| desktop_entry(&text, &file, &path))
            {
                apps.push(app);
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn desktop_files(
    root: &Path,
    directory: &Path,
    depth: usize,
    remaining: &mut usize,
    entries: &mut Vec<(String, PathBuf)>,
) {
    let Ok(files) = std::fs::read_dir(directory) else {
        return;
    };
    for file in files.flatten() {
        if *remaining == 0 {
            return;
        }
        *remaining -= 1;
        let path = file.path();
        if file.file_type().is_ok_and(|kind| kind.is_dir()) && depth > 0 {
            desktop_files(root, &path, depth - 1, remaining, entries);
        } else if path.extension().is_some_and(|ext| ext == "desktop") {
            if let Ok(relative) = path.strip_prefix(root) {
                entries.push((relative.to_string_lossy().replace('/', "-"), path));
            }
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn string_value(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        let ch = if ch == '\\' {
            match chars.next()? {
                's' => ' ',
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                '\\' => '\\',
                _ => return None,
            }
        } else {
            ch
        };
        if ch.is_control() {
            return None;
        }
        out.push(ch);
    }
    Some(out)
}

#[cfg(any(target_os = "linux", test))]
fn exec_words(text: &str) -> Option<Vec<String>> {
    let (mut words, mut word) = (Vec::new(), String::new());
    let (mut quoted, mut escaped, mut started) = (false, false, false);
    for ch in string_value(text)?.chars() {
        if ch.is_control() {
            return None;
        }
        if escaped {
            if !matches!(ch, '"' | '`' | '$' | '\\') {
                return None;
            }
            word.push(ch);
            escaped = false;
            started = true;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            quoted = !quoted;
            started = true;
        } else if ch.is_whitespace() && !quoted {
            if started {
                words.push(std::mem::take(&mut word));
                started = false;
            }
        } else {
            word.push(ch);
            started = true;
        }
    }
    if quoted || escaped {
        return None;
    }
    if started {
        words.push(word);
    }
    Some(words)
}

#[cfg(any(target_os = "linux", test))]
fn desktop_entry(text: &str, file: &Path, path: &[PathBuf]) -> Option<Value> {
    let mut fields = std::collections::BTreeMap::new();
    let mut main = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            main = line == "[Desktop Entry]";
        } else if main && !line.starts_with('#') {
            if let Some((key, value)) = line.split_once('=') {
                fields.insert(key, value);
            }
        }
    }
    if fields.get("Type") != Some(&"Application")
        || ["Hidden", "NoDisplay", "Terminal"]
            .iter()
            .any(|key| fields.get(key) == Some(&"true"))
    {
        return None;
    }
    // Recipes have no working-directory field; do not silently drop desktop semantics.
    if fields.get("Path").is_some_and(|value| !value.is_empty()) {
        return None;
    }
    let name = string_value(fields.get("Name")?)?;
    let mut words = exec_words(fields.get("Exec")?)?.into_iter();
    let resolve = |value: &str| {
        let direct = PathBuf::from(value);
        if direct.is_absolute() {
            crate::platform::is_executable(&direct).then_some(direct)
        } else {
            crate::platform::find_command(path.iter().cloned(), value)
        }
    };
    if let Some(value) = fields.get("TryExec") {
        resolve(&string_value(value)?)?;
    }
    let executable = resolve(&words.next()?)?;
    // Shell/env wrappers cannot identify the graphical process for duplicate-launch checks.
    if executable
        .file_name()
        .is_some_and(|name| matches!(name.to_str(), Some("env" | "sh" | "bash" | "zsh")))
    {
        return None;
    }
    let mut args = Vec::<String>::new();
    for word in words {
        match word.as_str() {
            "%f" | "%F" | "%u" | "%U" => (),
            "%c" => args.push(name.clone()),
            "%k" => args.push(file.to_string_lossy().into()),
            "%i" => {
                if let Some(icon) = fields.get("Icon") {
                    if !icon.is_empty() {
                        args.extend(["--icon".into(), string_value(icon)?]);
                    }
                }
            }
            _ => {
                let literal = word.replace("%%", "\0");
                if literal.contains('%') {
                    return None;
                }
                args.push(literal.replace('\0', "%"));
            }
        }
    }
    Some(json!({"id":file,"name":name,"path":executable,"args":args}))
}

#[cfg(windows)]
pub(super) fn windows(apps: &mut Vec<Value>) {
    use winreg::{
        enums::{
            HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
        },
        RegKey,
    };
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(
                "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths",
                KEY_READ | view,
            ) else {
                continue;
            };
            for name in root.enum_keys().take(1024).flatten() {
                let Some(value) = root
                    .open_subkey_with_flags(&name, KEY_READ | view)
                    .ok()
                    .and_then(|key| key.get_value::<String, _>("").ok())
                else {
                    continue;
                };
                let path = PathBuf::from(value.trim().trim_matches('"'));
                if path.is_absolute()
                    && path.is_file()
                    && path
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                {
                    apps.push(json!({"name":path.file_stem().unwrap_or_default().to_string_lossy(),"path":path}));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_entry_preserves_literal_arguments_and_ignores_non_application_entries() {
        let root =
            std::env::temp_dir().join(format!("am-desktop-entry-{}", crate::store::uuid_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join(if cfg!(windows) {
            "sample.exe"
        } else {
            "sample"
        });
        std::fs::write(&executable, b"fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let file = root.join("sample.desktop");
        let text = format!("[Desktop Entry]\nType=Application\nName=Sample app\nExec=\"{}\" --label \"two words\" %% %U %c\n", executable.to_string_lossy().replace('\\', "\\\\\\\\"));
        let app = desktop_entry(&text, &file, &[]).unwrap();
        assert_eq!(
            app["args"],
            json!(["--label", "two words", "%", "Sample app"])
        );
        for field in [
            "Hidden=true",
            "Terminal=true",
            "NoDisplay=true",
            "Exec=invalid %x",
            "Type=Link",
            "Path=/work",
        ] {
            assert!(desktop_entry(&format!("{text}{field}\n"), &file, &[]).is_none());
        }
        assert_eq!(
            exec_words(r#"sample "\\\\" "\\$""#),
            Some(vec!["sample".into(), "\\".into(), "$".into()])
        );
        assert_eq!(string_value(r"Two\sWords"), Some("Two Words".into()));
        assert!(string_value(r"bad\q").is_none());
        assert!(exec_words("sample \"unterminated").is_none());
        assert!(exec_words("sample trailing\\").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
