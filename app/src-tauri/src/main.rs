#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod engine;
mod import_links;
mod settings;
mod share_links;
mod startup;
mod theme;

use std::io::Write;
use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;
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

#[tauri::command]
fn autostart_get(app: tauri::AppHandle) -> Result<bool, String> {
    autostart::get(&app)
}

#[tauri::command]
fn autostart_set(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    autostart::set(&app, enabled)
}

#[tauri::command]
async fn share_links_preview(
    app: tauri::AppHandle,
) -> Result<Option<agent_manager_engine::desktop_links::Preview>, String> {
    tauri::async_runtime::spawn_blocking(move || share_links::preview(&app))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn share_links_register(
    app: tauri::AppHandle,
    base: String,
) -> Result<agent_manager_engine::desktop_links::Preview, String> {
    tauri::async_runtime::spawn_blocking(move || share_links::register(&app, &base))
        .await
        .map_err(|error| error.to_string())?
}

/// Restore geometry, but always open a visible, non-fullscreen window.
fn window_state_flags() -> StateFlags {
    StateFlags::POSITION | StateFlags::SIZE | StateFlags::MAXIMIZED
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = agent_manager_engine::environment_clock_helper_cli(&args) {
        std::process::exit(code);
    }
    if args.first().is_some_and(|arg| arg == "--account-launch") {
        let code = match agent_manager_engine::native_account_context::run_cli(&args[1..]) {
            Ok(code) => code,
            Err(err) => {
                eprintln!(
                    "{}",
                    agent_manager_engine::sanitize::sanitize_diagnostic_message(&format!(
                        "{err:#}"
                    ))
                );
                2
            }
        };
        std::process::exit(code);
    }
    let app = tauri::Builder::default()
        .manage(import_links::ImportLinkState::default())
        // 单实例须最先注册：二次启动时把已有主窗口唤到前台，不再起第二份进程。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(window_state_flags())
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let inspector = match app.path().resource_dir() {
                Ok(path) => Some(
                    path.join("claude-data-helper").join(format!(
                        "agent-manager-claude-data-helper{}",
                        std::env::consts::EXE_SUFFIX
                    )),
                ),
                Err(_) => {
                    let _ = writeln!(std::io::stderr(), "Read-only source inspection is unavailable: resource location was refused.");
                    None
                }
            };
            agent_manager_engine::configure_claude_data_inspection_helper(inspector);
            #[cfg(any(windows, target_os = "linux"))]
            {
                let helper = app
                    .path()
                    .resource_dir()?
                    .join("clock-helper")
                    .join(format!(
                        "agent-manager-clock-helper{}",
                        std::env::consts::EXE_SUFFIX
                    ));
                agent_manager_engine::configure_clock_helper(helper);
            }
            // 分享链接 agentmanager://import?data=…：只把链接文字交给界面的导入对话框，
            // 由界面校验、展示并等用户确认；这里不解析、不写任何东西。其他链接一律忽略。
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                let links = handle.state::<import_links::ImportLinkState>().record(
                    event
                        .urls()
                        .into_iter()
                        .map(|url| url.to_string())
                        .collect(),
                    false,
                );
                if !links.is_empty() {
                    show_main_window(&handle);
                    for link in links {
                        let _ = handle.emit("import-link", link);
                    }
                }
            });
            // The plugin reads Windows/Linux launch arguments before this setup
            // hook. Keep its snapshot for the frontend's listener-first read.
            let initial = app.deep_link().get_current()?.unwrap_or_default();
            app.state::<import_links::ImportLinkState>().record(
                initial.into_iter().map(|url| url.to_string()).collect(),
                true,
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            engine::engine_invoke,
            import_links::import_links_current,
            settings_get,
            settings_save,
            autostart_get,
            autostart_set,
            share_links_preview,
            share_links_register,
            theme::set_window_theme
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app_handle, event| match event {
        tauri::RunEvent::Exit => agent_manager_engine::shutdown_temporary_commands(),
        // macOS：点击 Dock 图标且没有可见窗口时恢复主窗口。
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } => show_main_window(_app_handle),
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
}
