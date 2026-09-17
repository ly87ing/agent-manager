#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engine;
mod settings;
mod theme;
mod tray;

use tauri::RunEvent;
use tauri_plugin_window_state::StateFlags;

#[tauri::command]
fn settings_get(app: tauri::AppHandle) -> Result<settings::SettingsData, String> {
    settings::settings_data(&app)
}

#[tauri::command]
fn settings_save(
    app: tauri::AppHandle,
    preferences: settings::SettingsPreferences,
) -> Result<settings::SettingsData, String> {
    settings::save_preferences(&app, preferences)
}

/// 跨会话持久化的窗口状态：位置/尺寸/最大化。
///
/// 有意不含 VISIBLE：关窗按钮被拦截为隐藏后，若用户从托盘「退出」，窗口处于
/// 隐藏态；持久化可见性会让下次启动恢复出一个隐藏窗口（只剩托盘，看起来像
/// 没启动）。不含 FULLSCREEN：不跨会话恢复全屏。
fn window_state_flags() -> StateFlags {
    StateFlags::POSITION | StateFlags::SIZE | StateFlags::MAXIMIZED
}

/// `RunEvent::ExitRequested` 的 code 语义（见 tauri-runtime-wry）：
/// 最后一个窗口被销毁 → code 为 None；app.exit(code)/app.restart() → Some(code)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitRequestAction {
    /// None：运行时自动触发（如隐藏窗口的 WebView 被回收导致无存活窗口），
    /// 阻止退出，保持托盘驻留。
    StayInTray,
    /// RESTART_EXIT_CODE：app.restart() / 自更新 relaunch。此路径上
    /// prevent_exit() 会被 Tauri 忽略（见 ExitRequestApi 文档），交还默认
    /// re-exec 流程：窗口状态由 window-state 插件的 RunEvent::Exit 钩子在
    /// 主线程保存，托盘图标由 Tauri 的 cleanup_before_exit 正常 Drop。
    DeferToTauriRestart,
    /// 其它 Some(_)：用户主动退出（托盘「退出」→ app.exit(0)）。
    CleanupAndExit,
}

fn classify_exit_request(code: Option<i32>) -> ExitRequestAction {
    match code {
        None => ExitRequestAction::StayInTray,
        Some(tauri::RESTART_EXIT_CODE) => ExitRequestAction::DeferToTauriRestart,
        Some(_) => ExitRequestAction::CleanupAndExit,
    }
}

fn main() {
    let app = tauri::Builder::default()
        // 单实例须最先注册：二次启动时把已有主窗口唤到前台，不再起第二份进程。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(window_state_flags())
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        // 关窗不退出：点击关闭按钮只隐藏窗口，驻留托盘；
        // 托盘「显示主界面」/ macOS Dock 点图标（Reopen）/ 二次启动均可唤回。
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            tray::setup(app)?;
            engine::maintenance_cleanup(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            engine::engine_invoke,
            settings_get,
            settings_save,
            theme::set_window_theme
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| match event {
        RunEvent::ExitRequested { api, code, .. } => match classify_exit_request(code) {
            ExitRequestAction::StayInTray => api.prevent_exit(),
            ExitRequestAction::DeferToTauriRestart => {}
            // 用户退出不拦截，让默认流程执行插件退出钩子（含窗口状态落盘）；
            // 仅显式摘托盘图标，避免进程结束后系统托盘残留死图标。
            ExitRequestAction::CleanupAndExit => tray::remove_tray_icon_before_exit(app_handle),
        },
        // macOS：点击 Dock 图标且没有可见窗口时恢复主窗口。
        #[cfg(target_os = "macos")]
        RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } => tray::show_main_window(app_handle),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_state_excludes_visibility_and_fullscreen() {
        let flags = window_state_flags();
        assert!(flags.contains(StateFlags::POSITION));
        assert!(flags.contains(StateFlags::SIZE));
        assert!(flags.contains(StateFlags::MAXIMIZED));
        assert!(!flags.contains(StateFlags::VISIBLE));
        assert!(!flags.contains(StateFlags::FULLSCREEN));
    }

    #[test]
    fn exit_request_classification() {
        assert_eq!(classify_exit_request(None), ExitRequestAction::StayInTray);
        assert_eq!(
            classify_exit_request(Some(tauri::RESTART_EXIT_CODE)),
            ExitRequestAction::DeferToTauriRestart
        );
        assert_eq!(
            classify_exit_request(Some(0)),
            ExitRequestAction::CleanupAndExit
        );
        assert_eq!(
            classify_exit_request(Some(1)),
            ExitRequestAction::CleanupAndExit
        );
    }
}
