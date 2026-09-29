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
const MENU_QUIT: &str = "quit";
const MENU_ACCOUNT_PREFIX: &str = "open_account:";

pub const TRAY_ACTION_EVENT: &str = "tray-action";

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
