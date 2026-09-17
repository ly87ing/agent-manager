//! 原生窗口主题（标题栏）同步。
//!
//! 前端契约：`invoke("set_window_theme", { theme })`，theme ∈ "light" | "dark" | "system"。

use tauri::{Manager, Theme};

/// "system" 映射为 None，让窗口跟随操作系统亮暗；其余取值报错而非静默吞掉，
/// 避免前端拼写漂移后标题栏与界面主题长期不一致却无人能发现。
pub(crate) fn parse_theme(theme: &str) -> Result<Option<Theme>, String> {
    match theme {
        "light" => Ok(Some(Theme::Light)),
        "dark" => Ok(Some(Theme::Dark)),
        "system" => Ok(None),
        other => Err(format!("非法主题 '{other}'（仅 light|dark|system）")),
    }
}

#[tauri::command]
pub fn set_window_theme(app: tauri::AppHandle, theme: String) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "主窗口不存在".to_string())?;
    window
        .set_theme(parse_theme(&theme)?)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_contract_values() {
        assert_eq!(parse_theme("light").unwrap(), Some(Theme::Light));
        assert_eq!(parse_theme("dark").unwrap(), Some(Theme::Dark));
        assert_eq!(parse_theme("system").unwrap(), None);
    }

    #[test]
    fn rejects_unknown_values() {
        for value in ["", "auto", "Light", "DARK", " system ", "systematic"] {
            assert!(parse_theme(value).is_err(), "{value:?} should fail");
        }
    }
}
