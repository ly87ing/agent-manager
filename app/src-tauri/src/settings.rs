use agent_manager_engine::{
    store::{self, AppSettings},
    Context,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::Manager;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SettingsPreferences {
    pub offline: bool,
    pub default_dry_run: bool,
    pub upstream_ttl_sec: u32,
    pub log_retention_days: u32,
}
impl From<AppSettings> for SettingsPreferences {
    fn from(v: AppSettings) -> Self {
        Self {
            offline: v.offline,
            default_dry_run: v.default_dry_run,
            upstream_ttl_sec: v.upstream_ttl_sec,
            log_retention_days: v.log_retention_days,
        }
    }
}
impl From<SettingsPreferences> for AppSettings {
    fn from(v: SettingsPreferences) -> Self {
        Self {
            offline: v.offline,
            default_dry_run: v.default_dry_run,
            upstream_ttl_sec: v.upstream_ttl_sec,
            log_retention_days: v.log_retention_days,
        }
    }
}

use std::sync::Mutex;

static STARTUP_MAINTENANCE_WARNING: Mutex<Option<String>> = Mutex::new(None);

pub fn set_startup_maintenance_warning(warning: Option<String>) {
    if let Ok(mut lock) = STARTUP_MAINTENANCE_WARNING.lock() {
        *lock = warning;
    }
}

pub fn get_startup_maintenance_warning() -> Option<String> {
    STARTUP_MAINTENANCE_WARNING
        .lock()
        .ok()
        .and_then(|g| g.clone())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsData {
    pub preferences: SettingsPreferences,
    pub app_version: String,
    pub contract_version: String,
    pub resource_dir: String,
    pub data_dir: String,
    pub log_dir: String,
    pub updater_configured: bool,
    pub maintenance_warning: Option<String>,
}

fn data_root(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    std::env::var_os("AGENT_MANAGER_DATA_DIR")
        .map(std::path::PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| app.path().app_data_dir().ok())
        .ok_or_else(|| "无法定位应用数据目录".into())
}
fn context(data_dir: &Path, home_dir: &Path) -> Context {
    Context {
        data_dir: data_dir.to_path_buf(),
        home_dir: home_dir.to_path_buf(),
    }
}
pub(crate) fn read_preferences_data_dir(
    data_dir: &Path,
    home_dir: &Path,
) -> Result<SettingsPreferences, String> {
    Ok(store::effective_settings(&context(data_dir, home_dir))
        .map_err(|e| e.to_string())?
        .into())
}
pub(crate) fn save_preferences_data_dir(
    data_dir: &Path,
    home_dir: &Path,
    preferences: SettingsPreferences,
) -> Result<(), String> {
    let mut conn = store::open_write(&context(data_dir, home_dir)).map_err(|e| e.to_string())?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    store::write_settings(&tx, &preferences.into()).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}
fn updater_configured(app: &tauri::AppHandle) -> bool {
    let Ok(value) = serde_json::to_value(app.config()) else {
        return false;
    };
    let Some(updater) = value.get("plugins").and_then(|p| p.get("updater")) else {
        return false;
    };
    let pubkey = updater.get("pubkey").and_then(|v| v.as_str()).unwrap_or("");
    let endpoint = updater
        .get("endpoints")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .unwrap_or("");
    pubkey.len() >= 40
        && !pubkey.starts_with("YOUR_")
        && pubkey
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/=_-".contains(c))
        && tauri::Url::parse(endpoint)
            .map(|u| u.scheme() == "https" && u.host().is_some())
            .unwrap_or(false)
}
pub(crate) fn current_preferences(app: &tauri::AppHandle) -> Result<SettingsPreferences, String> {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or("HOME 未设置")?;
    read_preferences_data_dir(&data_root(app)?, &home)
}
pub(crate) fn settings_data(app: &tauri::AppHandle) -> Result<SettingsData, String> {
    let data = data_root(app)?;
    let resource = app.path().resource_dir().map_err(|e| e.to_string())?;
    Ok(SettingsData {
        preferences: current_preferences(app)?,
        app_version: app
            .config()
            .version
            .clone()
            .unwrap_or_else(|| "unknown".into()),
        contract_version: crate::engine::CONTRACT_ID.into(),
        resource_dir: resource.display().to_string(),
        data_dir: data.display().to_string(),
        log_dir: data.join("logs").display().to_string(),
        updater_configured: updater_configured(app),
        maintenance_warning: get_startup_maintenance_warning(),
    })
}
pub(crate) fn save_preferences(
    app: &tauri::AppHandle,
    preferences: SettingsPreferences,
) -> Result<SettingsData, String> {
    let data = data_root(app)?;
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or("HOME 未设置")?;
    save_preferences_data_dir(&data, &home, preferences).map_err(|e| e.to_string())?;
    settings_data(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_read_without_database() {
        let root = std::env::temp_dir().join(format!("am-tauri-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(
            read_preferences_data_dir(&root.join("data"), &root)
                .unwrap()
                .upstream_ttl_sec,
            86400
        );
        assert_eq!(
            read_preferences_data_dir(&root.join("data"), &root)
                .unwrap()
                .log_retention_days,
            30
        );
        assert!(!root.join("data").exists());
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn preferences_save_reopens_and_camel_case_rejects_invalid_ttl() {
        let root =
            std::env::temp_dir().join(format!("am-tauri-settings-save-{}", std::process::id()));
        let data = root.join("data");
        let home = root.join("home");
        let value = SettingsPreferences {
            offline: true,
            default_dry_run: true,
            upstream_ttl_sec: 7,
            log_retention_days: 14,
        };
        save_preferences_data_dir(&data, &home, value.clone()).unwrap();
        assert_eq!(read_preferences_data_dir(&data, &home).unwrap(), value);
        assert!(serde_json::from_str::<SettingsPreferences>(
            r#"{"offline":false,"defaultDryRun":false,"upstreamTtlSec":-1,"logRetentionDays":30}"#
        )
        .is_err());
        assert!(serde_json::from_str::<SettingsPreferences>(
            r#"{"offline":false,"defaultDryRun":false,"upstreamTtlSec":1,"logRetentionDays":-1}"#
        )
        .is_err());
        assert!(serde_json::from_str::<SettingsPreferences>(
            r#"{"offline":false,"defaultDryRun":false,"upstreamTtlSec":1,"logRetentionDays":30,"extra":true}"#
        )
        .is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
