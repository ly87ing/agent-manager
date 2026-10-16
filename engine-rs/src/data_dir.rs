//! Shared CLI and desktop database selection; never moves or merges user data.

use std::path::{Path, PathBuf};

pub const APP_IDENTIFIER: &str = "io.github.ly87ing.agent-manager";

pub fn preferred(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    let root = home.join("Library/Application Support");
    #[cfg(windows)]
    let root = std::env::var_os("APPDATA")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData/Roaming"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let root = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    root.join(APP_IDENTIFIER)
}

pub fn resolve(preferred: Option<PathBuf>) -> Option<PathBuf> {
    select(
        preferred,
        std::env::var_os("AGENT_MANAGER_DATA_DIR").map(PathBuf::from),
    )
}

fn select(preferred: Option<PathBuf>, explicit: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = explicit.filter(|path| !path.as_os_str().is_empty()) {
        return Some(path);
    }
    let preferred = preferred?;
    // Separate application identifiers must not adopt the main app's legacy data.
    if preferred.file_name() == Some(std::ffi::OsStr::new(APP_IDENTIFIER)) {
        let legacy = preferred.with_file_name("agent-manager");
        if !preferred.join("native-manager.db").exists()
            && legacy.join("native-manager.db").exists()
        {
            return Some(legacy);
        }
    }
    Some(preferred)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_one_existing_database_without_creating_or_merging_data() {
        let root = std::env::temp_dir().join(format!("am-data-dir-{}", crate::store::uuid_v4()));
        let preferred = root.join(APP_IDENTIFIER);
        let legacy = root.join("agent-manager");
        assert_eq!(
            select(Some(preferred.clone()), None),
            Some(preferred.clone())
        );
        assert!(!root.exists());

        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("native-manager.db"), b"legacy").unwrap();
        assert_eq!(select(Some(preferred.clone()), None), Some(legacy.clone()));
        assert!(!preferred.exists());

        std::fs::create_dir_all(&preferred).unwrap();
        std::fs::write(preferred.join("native-manager.db"), b"desktop").unwrap();
        assert_eq!(
            select(Some(preferred.clone()), None),
            Some(preferred.clone())
        );
        let explicit = root.join("explicit");
        assert_eq!(
            select(Some(preferred.clone()), Some(explicit.clone())),
            Some(explicit.clone())
        );
        assert_eq!(select(None, Some(explicit.clone())), Some(explicit.clone()));
        assert_eq!(
            select(Some(preferred.clone()), Some(PathBuf::new())),
            Some(preferred.clone())
        );
        assert!(!explicit.exists());
        assert_eq!(
            std::fs::read(legacy.join("native-manager.db")).unwrap(),
            b"legacy"
        );
        assert_eq!(
            std::fs::read(preferred.join("native-manager.db")).unwrap(),
            b"desktop"
        );

        std::fs::remove_file(preferred.join("native-manager.db")).unwrap();
        let preview = root.join("YOUR_PREVIEW_IDENTIFIER");
        assert_eq!(select(Some(preview.clone()), None), Some(preview.clone()));
        assert!(!preview.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn identifier_matches_the_desktop_configuration() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../app/src-tauri/tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], APP_IDENTIFIER);
    }
}
