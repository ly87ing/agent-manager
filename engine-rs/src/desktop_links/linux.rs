use super::Preview;
use anyhow::{bail, Context, Result};
use std::{io::Write, path::PathBuf, process::Stdio, time::Duration};

const MIME: &str = "x-scheme-handler/agentmanager";

fn default_handler(text: &str, mime: &str) -> Result<String> {
    let first = text.lines().next().unwrap_or("");
    if first == format!("No default applications for “{mime}”") {
        return Ok(String::new());
    }
    first
        .strip_prefix(&format!("Default application for “{mime}”: "))
        .filter(|handler| !handler.is_empty() && !handler.chars().any(char::is_control))
        .map(str::to_string)
        .context("The desktop returned an unknown association response.")
}

pub struct Registration {
    home: PathBuf,
    data: PathBuf,
    config: PathBuf,
    executable: PathBuf,
    filename: String,
}

impl Registration {
    pub fn new(executable: PathBuf, identifier: &str) -> Result<Self> {
        let home =
            crate::platform::home_dir().context("Cannot find an absolute home directory.")?;
        let directory = |name: &str, fallback: &str| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(fallback))
        };
        if identifier.is_empty()
            || !identifier
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-_".contains(&c))
        {
            bail!("Invalid desktop application identifier.");
        }
        let data = directory("XDG_DATA_HOME", ".local/share");
        let config = directory("XDG_CONFIG_HOME", ".config");
        if !data.is_absolute() || !config.is_absolute() || !executable.is_absolute() {
            bail!("Desktop registration paths must be absolute.");
        }
        Ok(Self {
            home,
            data,
            config,
            executable,
            filename: format!("{identifier}-appimage.desktop"),
        })
    }

    fn path(&self) -> PathBuf {
        self.data.join("applications").join(&self.filename)
    }

    fn contents(&self) -> Result<Vec<u8>> {
        let path = self
            .executable
            .to_str()
            .filter(|path| !path.contains(['%', '=']) && !path.chars().any(char::is_control))
            .context("Move the AppImage to a path without %, = or control characters.")?;
        let mut escaped = String::new();
        for c in path.chars() {
            match c {
                '\\' => escaped.push_str("\\\\\\\\"),
                '"' | '`' | '$' => {
                    escaped.push_str("\\\\");
                    escaped.push(c);
                }
                _ => escaped.push(c),
            }
        }
        Ok(format!("[Desktop Entry]\nType=Application\nName=Agent Manager\nExec=\"{escaped}\" %u\nTerminal=false\nNoDisplay=true\nMimeType={MIME};\n").into_bytes())
    }

    fn mime(&self, args: &[&str]) -> Result<String> {
        // Older xdg-mime query implementations split Exec on whitespace without
        // parsing desktop quoting, rejecting valid AppImage paths. GIO uses the
        // same desktop-entry parser as actual URI launching.
        let query = match args {
            ["query", "default", mime] => Some(*mime),
            _ => None,
        };
        let mut command = crate::process::launch_command(if query.is_some() {
            "/usr/bin/gio"
        } else {
            "/usr/bin/xdg-mime"
        });
        let query_args;
        let args = if let Some(mime) = query {
            query_args = ["mime", mime];
            &query_args[..]
        } else {
            args
        };
        command
            .args(args)
            .env("HOME", &self.home)
            .env("XDG_DATA_HOME", &self.data)
            .env("XDG_CONFIG_HOME", &self.config)
            .env("LC_ALL", "C.UTF-8")
            .env("LANGUAGE", "C")
            .env_remove("CHARSET")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let output = crate::process::capture_group(&mut command, Duration::from_secs(8), 4096)?;
        if !output.status.success() || output.stdout.len() >= 4096 {
            bail!("The desktop could not read or update the share-link association.");
        }
        let value = String::from_utf8(output.stdout)?.trim().to_string();
        let value = if let Some(mime) = query {
            default_handler(&value, mime)?
        } else {
            value
        };
        if value.chars().any(char::is_control) {
            bail!("The desktop returned an invalid share-link association.");
        }
        Ok(value)
    }

    pub fn preview(&self) -> Result<Preview> {
        let expected = self.contents()?;
        let executable = std::fs::metadata(&self.executable)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if !executable.is_file() || executable.permissions().mode() & 0o111 == 0 {
                bail!("The AppImage must be an executable regular file.");
            }
        }
        let entry = self.path();
        let current = match std::fs::symlink_metadata(&entry) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
            Ok(meta) if meta.file_type().is_file() && meta.len() <= 8192 => {
                Some(std::fs::read(&entry)?)
            }
            Ok(_) => bail!(
                "The share-link desktop entry is not an owned regular file; it was not changed."
            ),
        };
        if current.as_ref().is_some_and(|bytes| bytes != &expected) {
            bail!(
                "The share-link desktop entry was changed or points elsewhere; review it manually."
            );
        }
        let handler = self.mime(&["query", "default", MIME])?;
        let base = crate::native_mcp::model::digest_of(&serde_json::json!({
            "entry": entry, "contents":current, "expected":expected,
            "handler":handler, "home":self.home,"config":self.config,
            "executable":self.executable,"size":executable.len(),"modified":executable.modified().ok()
        }));
        Ok(Preview {
            base,
            executable: self.executable.to_string_lossy().into(),
            entry: entry.to_string_lossy().into(),
            registered: current.is_some() && handler == self.filename,
            current_handler: handler,
        })
    }

    pub fn register(&self, base: &str) -> Result<Preview> {
        let current = self.preview()?;
        if current.base != base {
            bail!("Share-link settings changed. Review a new registration preview.");
        }
        if current.registered {
            return Ok(current);
        }
        let path = self.path();
        let expected = self.contents()?;
        if !path.exists() {
            std::fs::create_dir_all(path.parent().unwrap())?;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            file.write_all(&expected)?;
            file.sync_all()?;
        }
        // An unsuccessful association can leave this exact owned entry, making
        // retry safe. Never delete or rewrite the previous handler's entry.
        if self.preview()?.current_handler != current.current_handler {
            bail!("Share-link association changed. Review a new registration preview.");
        }
        self.mime(&["default", &self.filename, MIME])?;
        let actual = self.preview()?;
        if !actual.registered {
            bail!("Share-link registration was not verified. Read the current desktop association again.");
        }
        Ok(actual)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gio_default_response_preserves_ids_and_rejects_unknown_or_wrong_types() {
        assert_eq!(default_handler(&format!("Default application for “{MIME}”: foreign.desktop\nRegistered applications:\n\tother.desktop"),MIME).unwrap(),"foreign.desktop");
        assert_eq!(
            default_handler(&format!("No default applications for “{MIME}”"), MIME).unwrap(),
            ""
        );
        for text in [
            "",
            "localized or failed output",
            "No default applications for “text/plain”",
            "Default application for “text/plain”: foreign.desktop",
            "Default application for “x-scheme-handler/agentmanager”: ",
        ] {
            assert!(default_handler(text, MIME).is_err());
        }
    }
    #[test]
    fn desktop_entry_quotes_literal_paths_and_rejects_unusable_paths() {
        let mut registration = Registration {
            home: PathBuf::from("/YOUR_HOME"),
            data: PathBuf::from("/YOUR_DATA"),
            config: PathBuf::from("/YOUR_CONFIG"),
            executable: PathBuf::from("/opt/Agent \" $ ` \\ Manager.AppImage"),
            filename: "test.desktop".into(),
        };
        let entry = String::from_utf8(registration.contents().unwrap()).unwrap();
        assert!(entry.contains("\\\\\""));
        assert!(entry.contains("\\\\$"));
        assert!(entry.contains("\\\\`"));
        assert!(entry.contains("\\\\\\\\"));
        assert!(entry.contains(" %u\n"));
        for path in ["/opt/100%/app", "/opt/name=value/app", "/opt/new\nline/app"] {
            registration.executable = PathBuf::from(path);
            assert!(registration.contents().is_err());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_registration_preserves_foreign_entries_and_launches_literal_uri() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("am-links-{}", crate::store::uuid_v4()));
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        std::fs::create_dir(&root).unwrap();
        let _cleanup = Cleanup(root.clone());
        let home = root.join("home");
        let data = root.join("data");
        let config = root.join("config");
        let executable = root.join("Agent \" $ ` \\ Manager.AppImage");
        let marker = root.join("opened-uri");
        std::fs::write(
            &executable,
            format!("#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let registration = Registration {
            home: home.clone(),
            data: data.clone(),
            config: config.clone(),
            executable,
            filename: "org.example.links-appimage.desktop".into(),
        };
        let initial = registration.preview().unwrap();
        assert!(!initial.registered);
        assert!(!home.exists());
        assert!(!data.exists());
        assert!(!config.exists());
        std::fs::create_dir_all(data.join("applications")).unwrap();
        std::fs::create_dir(&config).unwrap();
        let foreign = data.join("applications/foreign.desktop");
        let bytes = b"[Desktop Entry]\nType=Application\nName=Foreign\nExec=/usr/bin/true %u\nMimeType=x-scheme-handler/agentmanager;text/plain;\n";
        std::fs::write(&foreign, bytes).unwrap();
        std::fs::write(config.join("mimeapps.list"), "[Default Applications]\nx-scheme-handler/agentmanager=foreign.desktop;\ntext/plain=foreign.desktop;\n").unwrap();
        assert!(registration.register(&initial.base).is_err());
        assert!(!registration.path().exists());
        let preview = registration.preview().unwrap();
        assert_eq!(preview.current_handler, "foreign.desktop");
        let actual = registration.register(&preview.base).unwrap();
        assert!(actual.registered);
        assert_eq!(std::fs::read(&foreign).unwrap(), bytes);
        assert_eq!(
            registration
                .mime(&["query", "default", "text/plain"])
                .unwrap(),
            "foreign.desktop"
        );
        assert!(registration.register(&actual.base).unwrap().registered);

        let uri = "agentmanager://import?data=literal%26value%3D%24HOME";
        let mut command = crate::process::launch_command("/usr/bin/gio");
        command
            .arg("launch")
            .arg(registration.path())
            .arg(uri)
            .env("HOME", &home)
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CONFIG_HOME", &config)
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/nonexistent-am-test-bus",
            )
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output =
            crate::process::capture_group(&mut command, Duration::from_secs(8), 4096).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), uri);

        std::fs::write(registration.path(), b"external replacement").unwrap();
        assert!(registration.register(&actual.base).is_err());
        assert_eq!(
            std::fs::read(registration.path()).unwrap(),
            b"external replacement"
        );
        std::fs::remove_file(registration.path()).unwrap();
        std::os::unix::fs::symlink(&foreign, registration.path()).unwrap();
        assert!(registration.preview().is_err());
        assert_eq!(std::fs::read(&foreign).unwrap(), bytes);
    }
}
