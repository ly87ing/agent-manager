//! 系统托盘：图标、账号面板、最小菜单，以及关窗驻留相关的共用入口。
//!
//! macOS / Windows 左键点图标弹出账号面板（一个小 webview 窗口，失焦即隐藏），右键是原生菜单；
//! Linux 的托盘收不到点击事件，左右键都是菜单。

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::menu::{Menu, MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, Rect, WebviewUrl, WebviewWindowBuilder,
};

/// 托盘图标 id；退出前通过 `AppHandle::tray_by_id` 取回句柄显式摘除。
pub const TRAY_ID: &str = "agent-manager-tray";

const MENU_SHOW_MAIN: &str = "show_main";
const MENU_SEARCH: &str = "open_search";
const MENU_QUIT: &str = "quit";
const MENU_ACCOUNT_PREFIX: &str = "open_account:";

pub const TRAY_ACTION_EVENT: &str = "tray-action";

/// 托盘面板窗口的 label；界面据此只渲染面板，不渲染主界面。
pub const PANEL_LABEL: &str = "tray-panel";
const PANEL_WIDTH: f64 = 380.0;
const PANEL_HEIGHT: f64 = 520.0;
/// 面板与图标、屏幕边缘的间距（逻辑像素）。
const PANEL_GAP: f64 = 6.0;

/// 界面发来的一个账号：只有 id、显示名、启动命令名与额度摘要，不含配置内容或凭据。
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrayAccount {
    agent_id: String,
    agent_name: String,
    account_id: String,
    account_name: String,
    command: Option<String>,
    usage_summary: Option<String>,
}

/// 点中账号：界面打开命令面板并定位到它，按 Enter 复制启动命令。托盘本身不写任何配置。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrayAccountAction {
    #[serde(rename = "type")]
    kind: &'static str,
    agent_name: String,
    account_id: String,
    account_name: String,
}

#[derive(Default)]
pub struct TrayState(Mutex<Vec<TrayAccount>>);

fn escape_menu_text(value: &str) -> String {
    value.replace('&', "&&")
}

/// 「名称 · 命令 · 额度」；还没有启动命令的账号标明，点开后去账号页处理。
fn account_menu_label(account: &TrayAccount) -> String {
    let mut label = escape_menu_text(&account.account_name);
    match &account.command {
        Some(command) => label.push_str(&format!(" · {}", escape_menu_text(command))),
        None => label.push_str("（没有启动命令）"),
    }
    if let Some(usage) = &account.usage_summary {
        label.push_str(&format!(" · {}", escape_menu_text(usage)));
    }
    label
}

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
        id if id.starts_with(MENU_ACCOUNT_PREFIX) => {
            let Some(index) = id
                .strip_prefix(MENU_ACCOUNT_PREFIX)
                .and_then(|value| value.parse::<usize>().ok())
            else {
                return;
            };
            let account = app
                .state::<TrayState>()
                .0
                .lock()
                .ok()
                .and_then(|accounts| accounts.get(index).cloned());
            if let Some(account) = account {
                show_main_window(app);
                let _ = app.emit(
                    TRAY_ACTION_EVENT,
                    TrayAccountAction {
                        kind: "open-account",
                        agent_name: account.agent_name,
                        account_id: account.account_id,
                        account_name: account.account_name,
                    },
                );
            }
        }
        MENU_QUIT => app.exit(0),
        _ => {}
    }
}

/// 面板左上角的位置（物理像素）：水平对着图标中心，夹在屏幕内；
/// 图标在屏幕上半（macOS 菜单栏）就放在图标下方，在下半（Windows 任务栏）就放在上方。
fn panel_position(
    icon: (f64, f64, f64, f64),
    panel: (f64, f64),
    screen: (f64, f64, f64, f64),
    gap: f64,
) -> (f64, f64) {
    let (ix, iy, iw, ih) = icon;
    let (pw, ph) = panel;
    let (sx, sy, sw, sh) = screen;
    let min_x = sx + gap;
    let max_x = (sx + sw - pw - gap).max(min_x);
    let x = (ix + iw / 2.0 - pw / 2.0).clamp(min_x, max_x);
    let y = if iy + ih / 2.0 < sy + sh / 2.0 {
        iy + ih + gap
    } else {
        iy - ph - gap
    };
    (x, y)
}

fn panel_window(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        return Ok(window);
    }
    WebviewWindowBuilder::new(app, PANEL_LABEL, WebviewUrl::App("index.html".into()))
        .title("Agent Manager")
        .inner_size(PANEL_WIDTH, PANEL_HEIGHT)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()
}

/// 点一下图标：面板开着就收起，否则放到图标旁边并显示。
fn toggle_panel(app: &AppHandle, icon: Rect) {
    let Ok(window) = panel_window(app) else {
        // 面板建不出来时退回主界面，至少能用
        show_main_window(app);
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    let position = icon.position.to_physical::<f64>(scale);
    let size = icon.size.to_physical::<f64>(scale);
    let monitor = app
        .monitor_from_point(position.x, position.y)
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    if let Some(monitor) = monitor {
        let area = monitor.work_area();
        let scale = monitor.scale_factor();
        let (x, y) = panel_position(
            (position.x, position.y, size.width, size.height),
            (PANEL_WIDTH * scale, PANEL_HEIGHT * scale),
            (
                area.position.x as f64,
                area.position.y as f64,
                area.size.width as f64,
                area.size.height as f64,
            ),
            PANEL_GAP * scale,
        );
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
    let _ = window.show();
    let _ = window.set_focus();
}

/// 面板失焦即收起，和系统托盘菜单的手感一致。
pub fn handle_panel_focus(window: &tauri::Window, focused: bool) {
    if !focused && window.label() == PANEL_LABEL {
        let _ = window.hide();
    }
}

fn hide_panel(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        let _ = window.hide();
    }
}

/// 面板里的「打开主界面」：收起面板，唤回主窗口。
#[tauri::command]
pub fn tray_panel_open_main(app: AppHandle) {
    hide_panel(&app);
    show_main_window(&app);
}

#[tauri::command]
pub fn tray_panel_quit(app: AppHandle) {
    app.exit(0);
}

fn build_menu(app: &AppHandle, accounts: &[TrayAccount]) -> tauri::Result<Menu<tauri::Wry>> {
    let show_main = MenuItem::with_id(app, MENU_SHOW_MAIN, "显示主界面", true, None::<&str>)?;
    let search = MenuItem::with_id(app, MENU_SEARCH, "搜索账号与资源…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "退出", true, None::<&str>)?;
    let mut menu = MenuBuilder::new(app).item(&show_main);

    let mut agent_ids: Vec<&str> = Vec::new();
    for account in accounts {
        if !agent_ids.contains(&account.agent_id.as_str()) {
            agent_ids.push(&account.agent_id);
        }
    }
    if !agent_ids.is_empty() {
        menu = menu.separator();
        for agent_id in agent_ids {
            let members: Vec<(usize, &TrayAccount)> = accounts
                .iter()
                .enumerate()
                .filter(|(_, account)| account.agent_id == agent_id)
                .collect();
            let agent_name = members
                .first()
                .map(|(_, account)| account.agent_name.as_str())
                .unwrap_or(agent_id);
            let mut submenu = SubmenuBuilder::new(app, escape_menu_text(agent_name));
            for (index, account) in members {
                let item = MenuItem::with_id(
                    app,
                    format!("{MENU_ACCOUNT_PREFIX}{index}"),
                    account_menu_label(account),
                    true,
                    None::<&str>,
                )?;
                submenu = submenu.item(&item);
            }
            menu = menu.item(&submenu.build()?);
        }
    }

    menu.separator()
        .item(&search)
        .separator()
        .item(&quit)
        .build()
}

#[tauri::command]
pub fn tray_set_accounts(app: AppHandle, accounts: Vec<TrayAccount>) -> Result<(), String> {
    let menu = build_menu(&app, &accounts).map_err(|error| error.to_string())?;
    let tray = app
        .tray_by_id(TRAY_ID)
        .ok_or_else(|| "托盘尚未初始化".to_string())?;
    let state = app.state::<TrayState>();
    let mut stored = state.0.lock().map_err(|_| "托盘状态锁已损坏".to_string())?;
    let previous = std::mem::replace(&mut *stored, accounts);
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
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                toggle_panel(tray.app_handle(), rect);
            }
        })
        // Linux 收不到托盘点击事件，左键也给菜单；其余平台左键开面板、右键开菜单
        .show_menu_on_left_click(cfg!(target_os = "linux"));
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

    fn account(command: Option<&str>, usage: Option<&str>) -> TrayAccount {
        TrayAccount {
            agent_id: "claude-code".into(),
            agent_name: "Claude Code".into(),
            account_id: "profiles:work".into(),
            account_name: "工作&A".into(),
            command: command.map(str::to_string),
            usage_summary: usage.map(str::to_string),
        }
    }

    #[test]
    fn account_labels_show_name_command_and_usage_without_config_values() {
        assert_eq!(
            account_menu_label(&account(Some("cc-work"), None)),
            "工作&&A · cc-work"
        );
        assert_eq!(
            account_menu_label(&account(Some("cc-work"), Some("5 小时已用 82%（缓存）"))),
            "工作&&A · cc-work · 5 小时已用 82%（缓存）"
        );
        assert_eq!(
            account_menu_label(&account(None, None)),
            "工作&&A（没有启动命令）"
        );
    }

    #[test]
    fn panel_sits_under_a_menu_bar_icon_and_above_a_taskbar_icon() {
        let screen = (0.0, 0.0, 1440.0, 900.0);
        // macOS：菜单栏图标在顶部，面板在它正下方、水平居中对齐
        assert_eq!(
            panel_position((700.0, 0.0, 40.0, 24.0), (380.0, 520.0), screen, 6.0),
            (530.0, 30.0)
        );
        // Windows：任务栏图标在底部，面板在它上方
        assert_eq!(
            panel_position((700.0, 860.0, 40.0, 40.0), (380.0, 520.0), screen, 6.0),
            (530.0, 334.0)
        );
        // 靠右边缘的图标：面板夹回屏幕内
        assert_eq!(
            panel_position((1420.0, 0.0, 20.0, 24.0), (380.0, 520.0), screen, 6.0),
            (1054.0, 30.0)
        );
        // 副屏在主屏左侧（负坐标）
        assert_eq!(
            panel_position(
                (-1500.0, 0.0, 20.0, 24.0),
                (380.0, 520.0),
                (-1920.0, 0.0, 1920.0, 1080.0),
                6.0
            ),
            (-1680.0, 30.0)
        );
    }

    #[test]
    fn account_action_carries_only_ids_and_names() {
        let action = TrayAccountAction {
            kind: "open-account",
            agent_name: "Claude Code".into(),
            account_id: "profiles:work".into(),
            account_name: "工作".into(),
        };
        assert_eq!(
            serde_json::to_value(action).unwrap(),
            serde_json::json!({"type": "open-account", "agentName": "Claude Code", "accountId": "profiles:work", "accountName": "工作"})
        );
    }
}
