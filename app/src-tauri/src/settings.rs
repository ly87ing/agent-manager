use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::Manager;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPreferences {
    pub offline: bool,
    pub default_dry_run: bool,
    pub upstream_ttl_sec: u32,
}

impl Default for SettingsPreferences {
    fn default() -> Self { Self { offline: false, default_dry_run: false, upstream_ttl_sec: 86_400 } }
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
}

#[derive(Debug, Serialize, Deserialize)]
struct SettingsFile { preferences: SettingsPreferences }

fn data_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    if let Some(raw) = std::env::var_os("AGENT_MANAGER_DATA_DIR") {
        if !raw.is_empty() { return Ok(PathBuf::from(raw)); }
    }
    app.path().app_data_dir().map_err(|e| format!("无法定位应用数据目录: {e}"))
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> { Ok(data_root(app)?.join("settings.json")) }

fn read_preferences_path(path: &Path) -> Result<SettingsPreferences, String> {
    if !path.is_file() { return Ok(SettingsPreferences::default()); }
    let bytes = std::fs::read(&path).map_err(|e| format!("读取设置失败: {e}"))?;
    let file: SettingsFile = serde_json::from_slice(&bytes).map_err(|e| format!("设置文件无效: {e}"))?;
    Ok(file.preferences)
}

fn read_preferences(app: &tauri::AppHandle) -> Result<SettingsPreferences, String> { read_preferences_path(&settings_path(app)?) }

fn updater_configured(app: &tauri::AppHandle) -> bool {
    let Ok(value) = serde_json::to_value(app.config()) else { return false; };
    let Some(updater) = value.get("plugins").and_then(|p| p.get("updater")) else { return false; };
    let pubkey = updater.get("pubkey").and_then(|v| v.as_str()).unwrap_or("");
    let endpoint = updater.get("endpoints").and_then(|v| v.as_array()).and_then(|a| a.first()).and_then(|v| v.as_str()).unwrap_or("");
    let pubkey_valid = pubkey.len() >= 40 && !pubkey.starts_with("YOUR_") && pubkey.chars().all(|c| c.is_ascii_alphanumeric() || "+/=_-".contains(c));
    let endpoint_valid = tauri::Url::parse(endpoint).map(|url| url.scheme() == "https" && url.host().is_some()).unwrap_or(false);
    pubkey_valid && endpoint_valid
}

pub(crate) fn current_preferences(app: &tauri::AppHandle) -> Result<SettingsPreferences, String> { read_preferences(app) }

pub(crate) fn settings_data(app: &tauri::AppHandle) -> Result<SettingsData, String> {
    let data = data_root(app)?;
    let resource = app.path().resource_dir().map_err(|e| format!("无法定位应用资源目录: {e}"))?;
    let log_dir = {
        #[cfg(debug_assertions)]
        { crate::engine::resolve_root().unwrap_or_else(|_| data.join("engine")).join("logs") }
        #[cfg(not(debug_assertions))]
        { data.join("engine/logs") }
    };
    Ok(SettingsData { preferences: read_preferences(app)?, app_version: app.config().version.clone().unwrap_or_else(|| "unknown".into()), contract_version: crate::engine::CONTRACT_ID.into(), resource_dir: resource.display().to_string(), data_dir: data.display().to_string(), log_dir: log_dir.display().to_string(), updater_configured: updater_configured(app) })
}

fn save_preferences_path(path: &Path, preferences: SettingsPreferences) -> Result<(), String> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| format!("创建设置目录失败: {e}"))?;
    let bytes = serde_json::to_vec_pretty(&SettingsFile { preferences }).map_err(|e| format!("编码设置失败: {e}"))?;
    let temp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&temp, bytes).map_err(|e| format!("写入设置失败: {e}"))?;
    std::fs::rename(temp, path).map_err(|e| format!("原子替换设置失败: {e}"))?;
    Ok(())
}

pub(crate) fn save_preferences(app: &tauri::AppHandle, preferences: SettingsPreferences) -> Result<SettingsData, String> {
    let path = settings_path(app)?;
    app.path().resource_dir().map_err(|e| format!("无法定位应用资源目录: {e}"))?;
    save_preferences_path(&path, preferences)?;
    settings_data(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf { std::env::temp_dir().join(format!("agent-manager-settings-{name}-{}", std::process::id())) }

    #[test]
    fn defaults_do_not_create_file_and_save_round_trips() {
        let path = temp_path("roundtrip"); let _ = std::fs::remove_file(&path);
        assert!(!path.exists());
        assert_eq!(read_preferences_path(&path).unwrap(), SettingsPreferences::default());
        let value = SettingsPreferences { offline: true, default_dry_run: true, upstream_ttl_sec: 7 };
        save_preferences_path(&path, value.clone()).unwrap();
        assert_eq!(read_preferences_path(&path).unwrap(), value);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn damaged_json_and_save_failure_are_errors() {
        let path = temp_path("damaged"); std::fs::write(&path, b"not-json").unwrap();
        assert!(read_preferences_path(&path).is_err());
        let parent = temp_path("file-parent"); std::fs::write(&parent, b"file").unwrap();
        assert!(save_preferences_path(&parent.join("settings.json"), SettingsPreferences::default()).is_err());
        let _ = std::fs::remove_file(path); let _ = std::fs::remove_file(parent);
    }

    #[test]
    fn ttl_is_u32_and_default_dry_run_is_not_execution_mode() {
        let value = SettingsPreferences::default();
        assert_eq!(value.upstream_ttl_sec, 86_400);
        assert!(!value.offline && !value.default_dry_run);
        let json = serde_json::json!({"preferences":{"offline":false,"defaultDryRun":false,"upstreamTtlSec":-1}});
        assert!(serde_json::from_value::<SettingsFile>(json).is_err());
    }
}
