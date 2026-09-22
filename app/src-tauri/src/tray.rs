//! 系统托盘：图标、最小菜单，以及关窗驻留相关的共用入口。

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::menu::{Menu, MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};

/// 托盘图标 id；退出前通过 `AppHandle::tray_by_id` 取回句柄显式摘除。
pub const TRAY_ID: &str = "agent-manager-tray";

const MENU_SHOW_MAIN: &str = "show_main";
const MENU_SEARCH: &str = "open_search";
const MENU_TAKEOVER: &str = "open_takeover";
const MENU_QUIT: &str = "quit";
const MENU_CONNECTION_PREFIX: &str = "switch_connection:";

/// 托盘发给界面的动作事件名；载荷是界面侧认识的动作 id。
pub const TRAY_ACTION_EVENT: &str = "tray-action";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrayConnection {
    agent_id: String,
    agent_name: String,
    profile: String,
    profile_label: String,
    current: bool,
    needs_repair: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrayConnectionAction {
    #[serde(rename = "type")]
    kind: &'static str,
    agent_id: String,
    agent_name: String,
    profile: String,
    profile_label: String,
}

#[derive(Default)]
pub struct TrayState(Mutex<Vec<TrayConnection>>);

fn escape_menu_text(value: &str) -> String {
    value.replace('&', "&&")
}

fn connection_menu_label(connection: &TrayConnection) -> String {
    if connection.current {
        format!("✓ {}", escape_menu_text(&connection.profile_label))
    } else if connection.needs_repair {
        format!(
            "⚠ {} · 重新应用",
            escape_menu_text(&connection.profile_label)
        )
    } else {
        escape_menu_text(&connection.profile_label)
    }
}

/// 恢复并聚焦主窗口；托盘「显示主界面」、单实例二次启动回调与
/// macOS Dock Reopen 共用同一条路径。
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn handle_menu_event(app: &AppHandle, event_id: &str) {
    match event_id {
        MENU_SHOW_MAIN => show_main_window(app),
        MENU_SEARCH => {
            show_main_window(app);
            let _ = app.emit(TRAY_ACTION_EVENT, "search");
        }
        MENU_TAKEOVER => {
            show_main_window(app);
            let _ = app.emit(TRAY_ACTION_EVENT, "takeover");
        }
        id if id.starts_with(MENU_CONNECTION_PREFIX) => {
            let Some(index) = id
                .strip_prefix(MENU_CONNECTION_PREFIX)
                .and_then(|value| value.parse::<usize>().ok())
            else {
                return;
            };
            let connection = app
                .state::<TrayState>()
                .0
                .lock()
                .ok()
                .and_then(|connections| connections.get(index).cloned());
            if let Some(connection) = connection {
                show_main_window(app);
                let _ = app.emit(
                    TRAY_ACTION_EVENT,
                    TrayConnectionAction {
                        kind: "switch-connection",
                        agent_id: connection.agent_id,
                        agent_name: connection.agent_name,
                        profile: connection.profile,
                        profile_label: connection.profile_label,
                    },
                );
            }
        }
        // 退出统一走 app.exit(0) → RunEvent::ExitRequested，由 main.rs 收尾摘托盘图标。
        MENU_QUIT => app.exit(0),
        _ => {}
    }
}

fn build_menu(app: &AppHandle, connections: &[TrayConnection]) -> tauri::Result<Menu<tauri::Wry>> {
    let show_main = MenuItem::with_id(app, MENU_SHOW_MAIN, "显示主界面", true, None::<&str>)?;
    let search = MenuItem::with_id(app, MENU_SEARCH, "搜索与执行…", true, None::<&str>)?;
    let takeover = MenuItem::with_id(app, MENU_TAKEOVER, "导入现有配置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "退出", true, None::<&str>)?;
    let mut menu = MenuBuilder::new(app).item(&show_main);

    let mut agent_ids: Vec<&str> = Vec::new();
    for connection in connections {
        if !agent_ids.contains(&connection.agent_id.as_str()) {
            agent_ids.push(&connection.agent_id);
        }
    }
    if !agent_ids.is_empty() {
        menu = menu.separator();
        for agent_id in agent_ids {
            let agent_connections: Vec<(usize, &TrayConnection)> = connections
                .iter()
                .enumerate()
                .filter(|(_, connection)| connection.agent_id == agent_id)
                .collect();
            let current_label = agent_connections
                .iter()
                .find(|(_, connection)| connection.current)
                .map(|(_, connection)| connection.profile_label.as_str())
                .unwrap_or("待确认");
            let agent_name = agent_connections
                .first()
                .map(|(_, connection)| connection.agent_name.as_str())
                .unwrap_or(agent_id);
            let submenu_label = escape_menu_text(&format!("{agent_name} · {current_label}"));
            let mut submenu = SubmenuBuilder::new(app, submenu_label);
            for (index, connection) in agent_connections {
                let label = connection_menu_label(connection);
                let item = MenuItem::with_id(
                    app,
                    format!("{MENU_CONNECTION_PREFIX}{index}"),
                    label,
                    !connection.current,
                    None::<&str>,
                )?;
                submenu = submenu.item(&item);
            }
            menu = menu.item(&submenu.build()?);
        }
    }

    menu.separator()
        .item(&search)
        .item(&takeover)
        .separator()
        .item(&quit)
        .build()
}

#[tauri::command]
pub fn tray_set_connections(
    app: AppHandle,
    connections: Vec<TrayConnection>,
) -> Result<(), String> {
    let menu = build_menu(&app, &connections).map_err(|error| error.to_string())?;
    let tray = app
        .tray_by_id(TRAY_ID)
        .ok_or_else(|| "托盘尚未初始化".to_string())?;
    let state = app.state::<TrayState>();
    let mut stored = state.0.lock().map_err(|_| "托盘状态锁已损坏".to_string())?;
    let previous = std::mem::replace(&mut *stored, connections);
    if let Err(error) = tray.set_menu(Some(menu)) {
        *stored = previous;
        return Err(error.to_string());
    }
    Ok(())
}

/// icons/32x32.png 是彩色应用图标而非 macOS 单色模板图，不设 `icon_as_template`
///（否则状态栏会渲染成纯黑剪影）；彩色图标在深浅色菜单栏下均可读。
fn tray_icon(app: &tauri::App) -> Option<tauri::image::Image<'static>> {
    const ICON_BYTES: &[u8] = include_bytes!("../icons/32x32.png");
    tauri::image::Image::from_bytes(ICON_BYTES)
        .ok()
        .or_else(|| {
            // default_window_icon 的借用绑在 &App 上，拷贝成 owned 再交给托盘。
            let icon = app.default_window_icon()?;
            Some(tauri::image::Image::new_owned(
                icon.rgba().to_vec(),
                icon.width(),
                icon.height(),
            ))
        })
}

pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    let menu = build_menu(app.handle(), &[])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Agent Manager")
        .menu(&menu)
        .on_menu_event(|app, event| handle_menu_event(app, event.id.as_ref()))
        .show_menu_on_left_click(true);
    // 内嵌 PNG 解码失败时退到打包默认窗口图标；两者皆无只打诊断、不阻断启动。
    match tray_icon(app) {
        Some(icon) => builder = builder.icon(icon),
        None => eprintln!("tray icon unavailable; building tray without icon"),
    }
    builder.build(app)?;
    Ok(())
}

/// 退出前显式摘除托盘图标。进程若绕过 `TrayIcon::drop`（如 std::process::exit），
/// Windows Shell 收不到 NIM_DELETE，会残留一个悬停才消失的死图标；
/// `set_visible(false)` 在各平台都是正常的隐藏/移除语义，作为统一收尾。
pub fn remove_tray_icon_before_exit(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_visible(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(current: bool, needs_repair: bool) -> TrayConnection {
        TrayConnection {
            agent_id: "claude-code".into(),
            agent_name: "Claude Code".into(),
            profile: "kimi".into(),
            profile_label: "Kimi Code".into(),
            current,
            needs_repair,
        }
    }

    #[test]
    fn connection_labels_distinguish_confirmed_current_and_repair() {
        assert_eq!(
            connection_menu_label(&connection(true, false)),
            "✓ Kimi Code"
        );
        assert_eq!(
            connection_menu_label(&connection(false, true)),
            "⚠ Kimi Code · 重新应用"
        );
        assert_eq!(
            connection_menu_label(&connection(false, false)),
            "Kimi Code"
        );
        let mut with_mnemonic = connection(false, false);
        with_mnemonic.profile_label = "A&B".into();
        assert_eq!(connection_menu_label(&with_mnemonic), "A&&B");
    }
}
