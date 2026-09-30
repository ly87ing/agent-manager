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
    pub log_retention_days: u32,
}
impl From<AppSettings> for SettingsPreferences {
    fn from(v: AppSettings) -> Self {
        Self {
            log_retention_days: v.log_retention_days,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsData {
    pub preferences: SettingsPreferences,
    pub app_version: String,
    pub contract_version: String,
    pub resource_dir: String,
    pub data_dir: String,
    pub updater_configured: bool,
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
    let ctx = context(data_dir, home_dir);
    let mut conn = store::open_write(&ctx).map_err(|e| e.to_string())?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    // 设置页只拥有日志保留天数；在同一写事务里读取，避免覆盖密钥页刚保存的路径。
    let mut settings = store::settings_from_connection(&tx).map_err(|e| e.to_string())?;
    settings.log_retention_days = preferences.log_retention_days;
    store::write_settings(&tx, &settings).map_err(|e| e.to_string())?;
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
    let home = agent_manager_engine::platform::home_dir()
        .ok_or("找不到用户主目录（HOME / USERPROFILE 都没设置）")?;
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
        updater_configured: updater_configured(app),
    })
}
pub(crate) fn save_preferences(
    app: &tauri::AppHandle,
    preferences: SettingsPreferences,
) -> Result<SettingsData, String> {
    let data = data_root(app)?;
    let home = agent_manager_engine::platform::home_dir()
        .ok_or("找不到用户主目录（HOME / USERPROFILE 都没设置）")?;
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
                .log_retention_days,
            30
        );
        assert!(!root.join("data").exists());
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn preferences_save_reopens_and_camel_case_rejects_unknown_fields() {
        let root =
            std::env::temp_dir().join(format!("am-tauri-settings-save-{}", std::process::id()));
        let data = root.join("data");
        let home = root.join("home");
        let value = SettingsPreferences {
            log_retention_days: 14,
        };
        save_preferences_data_dir(&data, &home, value.clone()).unwrap();
        assert_eq!(read_preferences_data_dir(&data, &home).unwrap(), value);
        assert!(serde_json::from_str::<SettingsPreferences>(r#"{"logRetentionDays":-1}"#).is_err());
        assert!(serde_json::from_str::<SettingsPreferences>(
            r#"{"logRetentionDays":30,"extra":true}"#
        )
        .is_err());
        // 已退役的偏好（defaultDryRun / offline / upstreamTtlSec）作为未知字段拒绝
        for raw in [
            r#"{"logRetentionDays":30,"defaultDryRun":false}"#,
            r#"{"logRetentionDays":30,"offline":false}"#,
            r#"{"logRetentionDays":30,"upstreamTtlSec":1}"#,
        ] {
            assert!(serde_json::from_str::<SettingsPreferences>(raw).is_err());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preferences_save_preserves_secret_file_path_owned_by_secrets_page() {
        let root = std::env::temp_dir().join(format!(
            "am-tauri-settings-preserve-secret-path-{}",
            std::process::id()
        ));
        let data = root.join("data");
        let home = root.join("home");
        let context = context(&data, &home);
        let conn = store::open_write(&context).unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        store::write_settings(
            &tx,
            &AppSettings {
                log_retention_days: 30,
                secrets_file_path: "~/private/secrets.env".into(),
            },
        )
        .unwrap();
        tx.commit().unwrap();

        save_preferences_data_dir(
            &data,
            &home,
            SettingsPreferences {
                log_retention_days: 14,
            },
        )
        .unwrap();

        let saved = store::effective_settings(&context).unwrap();
        assert_eq!(saved.log_retention_days, 14);
        assert_eq!(saved.secrets_file_path, "~/private/secrets.env");
        let _ = std::fs::remove_dir_all(root);
    }
}
